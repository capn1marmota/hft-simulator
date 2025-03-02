use tokio::sync::mpsc;
use crate::order_book::{Order, OrderBook, OrderType, OrderSide};
use std::sync::Arc;
use log::{warn, info};
use crate::RiskManager;

pub enum EngineMessage {
    NewOrder(Order),
    CancelOrder { symbol: String, order_id: u64 },
}

pub struct MatchingEngine {
    order_book: Arc<OrderBook>,
    risk_manager: Arc<RiskManager>,
    message_rx: mpsc::UnboundedReceiver<EngineMessage>,
}

impl MatchingEngine {
    pub fn new(
        order_book: Arc<OrderBook>,
        risk_manager: Arc<RiskManager>
    ) -> (Self, mpsc::UnboundedSender<EngineMessage>) {
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        (Self { order_book, message_rx, risk_manager }, message_tx)
    }

    pub async fn run(mut self) {
        while let Some(msg) = self.message_rx.recv().await {
            match msg {
                EngineMessage::NewOrder(order) => {
                    self.process_order(order).await;
                }
                EngineMessage::CancelOrder { symbol, order_id } => {
                    self.process_cancellation(&symbol, order_id).await;
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
        let remaining_qty = order.quantity;
        let mut order = order.clone();

        info!("Processing order {}: {:?}", order.id, order);
    
        match order.order_type {
            OrderType::Limit if order.price > 0.0 => {
                let is_bid = matches!(order.side, OrderSide::Buy);
                self.process_limit_order(&symbol, &order, remaining_qty, is_bid).await
            }
            OrderType::Market => {
                let is_bid = matches!(order.side, OrderSide::Buy);
                self.process_market_order(&symbol, &order, remaining_qty, is_bid).await
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
        mut remaining_qty: f64,
        is_bid: bool,
    ) {
        if is_bid {
            self.match_bid_order(symbol, order, &mut remaining_qty).await
        } else {
            self.match_ask_order(symbol, order, &mut remaining_qty).await
        }

        if remaining_qty > 0.0 {
            let mut new_order = order.clone();
            new_order.quantity = remaining_qty;
            self.order_book.add_order(new_order);
        }
    }

    async fn match_bid_order(&self, symbol: &str, order: &Order, remaining_qty: &mut f64) {
        if let Some(mut asks) = self.order_book.asks.get_mut(symbol) {
            let mut to_remove = Vec::new();
            
            for (price_key, orders) in asks.iter_mut() {
                let ask_price = price_key.into_inner();
                if ask_price > order.price {
                    break;
                }

                let mut filled_indices = Vec::new();
                for (idx, ask_order) in orders.iter_mut().enumerate() {
                    let fill_qty = (*remaining_qty).min(ask_order.quantity);
                    self.record_transaction(symbol, ask_price, fill_qty, OrderSide::Buy);
                    
                    *remaining_qty -= fill_qty;
                    ask_order.quantity -= fill_qty;

                    if ask_order.quantity <= 0.0 {
                        filled_indices.push(idx);
                    }
                    if *remaining_qty <= 0.0 {
                        break;
                    }
                }

                self.cleanup_filled_orders(orders, &filled_indices);
                if orders.is_empty() {
                    to_remove.push(*price_key);
                }
                if *remaining_qty <= 0.0 {
                    break;
                }
            }

            for price in to_remove {
                asks.remove(&price);
            }
        }
    }

    async fn match_ask_order(&self, symbol: &str, order: &Order, remaining_qty: &mut f64) {
        if let Some(mut bids) = self.order_book.bids.get_mut(symbol) {
            let mut to_remove = Vec::new();
            
            for (price_key, orders) in bids.iter_mut() {
                let bid_price = price_key.0.into_inner();
                if bid_price < order.price {
                    break;
                }

                let mut filled_indices = Vec::new();
                for (idx, bid_order) in orders.iter_mut().enumerate() {
                    let fill_qty = (*remaining_qty).min(bid_order.quantity);
                    self.record_transaction(symbol, bid_price, fill_qty, OrderSide::Sell);
                    
                    *remaining_qty -= fill_qty;
                    bid_order.quantity -= fill_qty;

                    if bid_order.quantity <= 0.0 {
                        filled_indices.push(idx);
                    }
                    if *remaining_qty <= 0.0 {
                        break;
                    }
                }

                self.cleanup_filled_orders(orders, &filled_indices);
                if orders.is_empty() {
                    to_remove.push(*price_key);
                }
                if *remaining_qty <= 0.0 {
                    break;
                }
            }

            for price in to_remove {
                bids.remove(&price);
            }
        }
    }

    async fn process_market_order(
        &self,
        symbol: &str,
        order: &Order,
        mut remaining_qty: f64,
        is_bid: bool,
    ) {
        if is_bid {
            self.process_market_buy(symbol, order, &mut remaining_qty).await
        } else {
            self.process_market_sell(symbol, order, &mut remaining_qty).await
        }
    }

    async fn process_market_buy(&self, symbol: &str, _order: &Order, remaining_qty: &mut f64) {
        if let Some(mut asks) = self.order_book.asks.get_mut(symbol) {
            let mut to_remove = Vec::new();
            
            for (price_key, orders) in asks.iter_mut() {
                let ask_price = price_key.into_inner();
                let mut filled_indices = Vec::new();

                for (idx, ask_order) in orders.iter_mut().enumerate() {
                    let fill_qty = (*remaining_qty).min(ask_order.quantity);
                    self.record_transaction(symbol, ask_price, fill_qty, OrderSide::Buy);
                    
                    *remaining_qty -= fill_qty;
                    ask_order.quantity -= fill_qty;

                    if ask_order.quantity <= 0.0 {
                        filled_indices.push(idx);
                    }
                    if *remaining_qty <= 0.0 {
                        break;
                    }
                }

                self.cleanup_filled_orders(orders, &filled_indices);
                if orders.is_empty() {
                    to_remove.push(*price_key);
                }
                if *remaining_qty <= 0.0 {
                    break;
                }
            }

            for price in to_remove {
                asks.remove(&price);
            }
        }
    }

    async fn process_market_sell(&self, symbol: &str, _order: &Order, remaining_qty: &mut f64) {
        if let Some(mut bids) = self.order_book.bids.get_mut(symbol) {
            let mut to_remove = Vec::new();
            
            for (price_key, orders) in bids.iter_mut() {
                let bid_price = price_key.0.into_inner();
                let mut filled_indices = Vec::new();

                for (idx, bid_order) in orders.iter_mut().enumerate() {
                    let fill_qty = (*remaining_qty).min(bid_order.quantity);
                    self.record_transaction(symbol, bid_price, fill_qty, OrderSide::Sell);
                    
                    *remaining_qty -= fill_qty;
                    bid_order.quantity -= fill_qty;

                    if bid_order.quantity <= 0.0 {
                        filled_indices.push(idx);
                    }
                    if *remaining_qty <= 0.0 {
                        break;
                    }
                }

                self.cleanup_filled_orders(orders, &filled_indices);
                if orders.is_empty() {
                    to_remove.push(*price_key);
                }
                if *remaining_qty <= 0.0 {
                    break;
                }
            }

            for price in to_remove {
                bids.remove(&price);
            }
        }
    }

    fn cleanup_filled_orders(&self, orders: &mut Vec<Order>, filled_indices: &[usize]) {
        for idx in filled_indices.iter().rev() {
            orders.remove(*idx);
        }
    }

    fn record_transaction(&self, symbol: &str, price: f64, quantity: f64, side: OrderSide) {
        if quantity > 0.0 {
            self.risk_manager.record_transaction(symbol, price, quantity, side);
        }
    }
}