use tokio::sync::mpsc;
use crate::order_book::{Order, OrderBook, OrderType, OrderSide};
use std::sync::Arc;
use log::{warn, info};
use crate::RiskManager;
use ordered_float::OrderedFloat;
use std::cmp::Reverse;

pub enum EngineMessage {
    NewOrder(Order),
    CancelOrder { symbol: String, order_id: u64 },
}

#[derive(Clone)]
pub struct MatchingEngine {
    order_book: Arc<OrderBook>,
    risk_manager: Arc<RiskManager>,
}

impl MatchingEngine {
    pub fn report_positions(&self) {
        let get_price = |symbol: &str| {
            self.order_book.get_best_bid(symbol)
                .zip(self.order_book.get_best_ask(symbol))
                .map(|(bid, ask)| (bid + ask) / 2.0)
        };

        self.risk_manager.report_positions(get_price);
    }

    pub fn new(
        order_book: Arc<OrderBook>,
        risk_manager: Arc<RiskManager>
    ) -> (Self, mpsc::UnboundedSender<EngineMessage>, mpsc::UnboundedReceiver<EngineMessage>) {
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        (Self { order_book, risk_manager }, message_tx, message_rx)
    }

    pub async fn run(self, mut message_rx: mpsc::UnboundedReceiver<EngineMessage>) {
        while let Some(msg) = message_rx.recv().await {
            match msg {
                EngineMessage::NewOrder(order) => self.process_order(order).await,
                EngineMessage::CancelOrder { symbol, order_id } => {
                    self.process_cancellation(&symbol, order_id).await
                }
            }
        }
    }

    async fn process_cancellation(&self, symbol: &str, order_id: u64) {
        if self.order_book.cancel_order(symbol, order_id) {
            info!("Cancelled order {}", order_id);
        } else {
            warn!("Failed to cancel order {}", order_id);
        }
    }

    async fn process_order(&self, order: Order) {
        let symbol = order.symbol.clone();
        let mut remaining_qty = order.quantity;

        info!("Processing order {}: {:?}", order.id, order);

        match order.order_type {
            OrderType::Limit if order.price > 0.0 => {
                let is_bid = matches!(order.side, OrderSide::Buy);
                self.process_limit_order(&symbol, &order, &mut remaining_qty, is_bid).await;
            }
            OrderType::Market => {
                let is_bid = matches!(order.side, OrderSide::Buy);
                self.process_market_order(&symbol, &order, &mut remaining_qty, is_bid).await;
            }
            _ => warn!("Invalid order type/price"),
        }

        if remaining_qty > 0.0 {
            let mut new_order = order.clone();
            new_order.quantity = remaining_qty;
            self.order_book.add_order(new_order);
        }
    }

    async fn process_limit_order(
        &self,
        symbol: &str,
        order: &Order,
        remaining_qty: &mut f64,
        is_bid: bool,
    ) {
        if is_bid {
            self.match_orders(
                symbol,
                remaining_qty,
                order.price,
                |level_price, order_price| level_price <= order_price,
                OrderSide::Buy,
            ).await;
        } else {
            self.match_orders(
                symbol,
                remaining_qty,
                order.price,
                |level_price, order_price| level_price >= order_price,
                OrderSide::Sell,
            ).await;
        }
    }

    async fn process_market_order(
        &self,
        symbol: &str,
        _order: &Order,
        remaining_qty: &mut f64,
        is_bid: bool,
    ) {
        if is_bid {
            self.match_orders(
                symbol,
                remaining_qty,
                0.0, // dummy value; price_check always returns true for market orders
                |_, _| true,
                OrderSide::Buy,
            ).await;
        } else {
            self.match_orders(
                symbol,
                remaining_qty,
                0.0,
                |_, _| true,
                OrderSide::Sell,
            ).await;
        }
    }

    async fn match_orders<F>(
        &self,
        symbol: &str,
        remaining_qty: &mut f64,
        order_price: f64,
        price_check: F,
        fill_side: OrderSide,
    )
    where
        F: Fn(f64, f64) -> bool,
    {
        if fill_side == OrderSide::Buy {
            if let Some(mut asks) = self.order_book.asks.get_mut(symbol) {
                let mut levels_to_remove = Vec::new();
                // Iterate over ask levels (assumed sorted in ascending order)
                for (price_key, orders) in asks.iter_mut() {
                    let level_price = price_key.into_inner();
                    if order_price > 0.0 && !price_check(level_price, order_price) {
                        break;
                    }
                    // Pass a reference to fill_side instead of moving it.
                    Self::fill_order_level(symbol, level_price, orders, remaining_qty, &fill_side, &self.risk_manager);
                    if orders.is_empty() {
                        levels_to_remove.push(level_price);
                    }
                    if *remaining_qty <= 0.0 {
                        break;
                    }
                }
                for price in levels_to_remove {
                    asks.remove(&OrderedFloat::from(price));
                }
            }
        } else {
            if let Some(mut bids) = self.order_book.bids.get_mut(symbol) {
                let mut levels_to_remove = Vec::new();
                // Iterate over bid levels (assumed sorted in descending order)
                for (price_key, orders) in bids.iter_mut() {
                    let level_price = price_key.0.into_inner();
                    if order_price > 0.0 && !price_check(level_price, order_price) {
                        break;
                    }
                    // Pass a reference to fill_side
                    Self::fill_order_level(symbol, level_price, orders, remaining_qty, &fill_side, &self.risk_manager);
                    if orders.is_empty() {
                        levels_to_remove.push(level_price);
                    }
                    if *remaining_qty <= 0.0 {
                        break;
                    }
                }
                for price in levels_to_remove {
                    bids.remove(&Reverse(OrderedFloat::from(price)));
                }
            }
        }
    }

    // The helper function now borrows fill_side.
    fn fill_order_level(
        symbol: &str,
        level_price: f64,
        orders: &mut Vec<Order>,
        remaining_qty: &mut f64,
        fill_side: &OrderSide,
        risk_manager: &RiskManager,
    ) {
        let mut filled_indices = Vec::new();
        for (idx, existing_order) in orders.iter_mut().enumerate() {
            if *remaining_qty <= 0.0 {
                break;
            }
            let fill_qty = (*remaining_qty).min(existing_order.quantity);
            // Clone fill_side here because record_transaction requires ownership.
            risk_manager.record_transaction(symbol, level_price, fill_qty, fill_side.clone());
            *remaining_qty -= fill_qty;
            existing_order.quantity -= fill_qty;
            if existing_order.quantity <= 0.0 {
                filled_indices.push(idx);
            }
        }
        // Remove fully filled orders in reverse order to avoid index shifting.
        for idx in filled_indices.iter().rev() {
            orders.remove(*idx);
        }
    }

    #[allow(dead_code)]
    pub async fn start_reporting(self: Arc<Self>, interval_secs: u64) {
        let engine = Arc::new(self.clone());
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                engine.report_positions();
            }
        });
    }
}
