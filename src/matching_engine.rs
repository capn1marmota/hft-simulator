use crate::order_book::{Order, OrderBook, OrderSide, OrderType};
use crate::risk_management::RiskManager;
use log::{info, warn};
use rust_decimal::Decimal;
use std::cmp::Reverse;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Trade {
    pub id: u64,
    pub symbol: String,
    pub price: Decimal,
    pub quantity: Decimal,
    pub buyer_id: u64,
    pub seller_id: u64,
    pub timestamp: i64,
}

#[allow(dead_code)]
pub enum EngineMessage {
    NewOrder(Order),
    CancelOrder { symbol: String, order_id: u64 },
    BatchOrders(Vec<Order>),
}

#[derive(Clone)]
pub struct MatchingEngine {
    order_book: Arc<OrderBook>,
    risk_manager: Arc<RiskManager>,
    metrics: Arc<EngineMetrics>,
}

pub struct EngineMetrics {
    orders_processed: std::sync::atomic::AtomicU64,
    trades_executed: std::sync::atomic::AtomicU64,
    last_processing_time: std::sync::Mutex<Option<std::time::Duration>>,
}

impl EngineMetrics {
    fn new() -> Self {
        Self {
            orders_processed: std::sync::atomic::AtomicU64::new(0),
            trades_executed: std::sync::atomic::AtomicU64::new(0),
            last_processing_time: std::sync::Mutex::new(None),
        }
    }

    fn inc_orders_processed(&self) {
        self.orders_processed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn inc_trades_executed(&self, count: u64) {
        self.trades_executed
            .fetch_add(count, std::sync::atomic::Ordering::Relaxed);
    }

    fn set_processing_time(&self, duration: std::time::Duration) {
        let mut guard = self.last_processing_time.lock().unwrap();
        *guard = Some(duration);
    }

    fn report(&self) {
        let orders = self
            .orders_processed
            .load(std::sync::atomic::Ordering::Relaxed);
        let trades = self
            .trades_executed
            .load(std::sync::atomic::Ordering::Relaxed);
        let time = self.last_processing_time.lock().unwrap();

        let last_order_time = match *time {
            Some(duration) => format!("{:?}", duration),
            None => "N/A".to_string(),
        };

        info!(
            "Engine metrics | Orders: {} | Trades: {} | Last order processing time: {}",
            orders, trades, last_order_time
        );
    }
}

impl MatchingEngine {
    pub fn report_positions(&self) {
        let get_price = |symbol: &str| self.order_book.get_mid_price(symbol);
        self.risk_manager.report_positions(get_price);
        self.metrics.report();
    }

    pub fn new(
        order_book: Arc<OrderBook>,
        risk_manager: Arc<RiskManager>,
    ) -> (
        Self,
        mpsc::UnboundedSender<EngineMessage>,
        mpsc::UnboundedReceiver<EngineMessage>,
    ) {
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        (
            Self {
                order_book,
                risk_manager,
                metrics: Arc::new(EngineMetrics::new()),
            },
            message_tx,
            message_rx,
        )
    }

    pub async fn run(&self, message_rx: &mut mpsc::UnboundedReceiver<EngineMessage>) {
        while let Some(msg) = message_rx.recv().await {
            match msg {
                EngineMessage::NewOrder(order) => {
                    self.process_order(order).await;
                }
                EngineMessage::CancelOrder { symbol, order_id } => {
                    self.process_cancellation(&symbol, order_id).await;
                }
                EngineMessage::BatchOrders(orders) => {
                    for order in orders {
                        self.process_order(order).await;
                    }
                }
            }
        }
    }

    async fn process_order(&self, order: Order) {
        let start_time = Instant::now();
        self.metrics.inc_orders_processed();

        let _symbol = order.symbol.clone();
        info!("Processing order {}: {:?}", order.id, order);

        let trades = match order.order_type {
            OrderType::Limit if order.price > Decimal::ZERO => self.match_limit_order(&order),
            OrderType::Market => self.match_market_order(&order),
            _ => {
                warn!("Invalid order type/price");
                Vec::new()
            }
        };

        // Record trades for risk management
        for trade in &trades {
            let trade_side = if trade.buyer_id == order.id {
                OrderSide::Buy
            } else {
                OrderSide::Sell
            };

            self.risk_manager.record_transaction(
                &trade.symbol,
                trade.price,
                trade.quantity,
                trade_side,
            );
        }

        self.metrics.inc_trades_executed(trades.len() as u64);

        // Add remaining order to order book if it's a limit order
        let remaining_qty = order.quantity - trades.iter().map(|t| t.quantity).sum::<Decimal>();

        if remaining_qty > Decimal::new(1, 3) && order.order_type == OrderType::Limit {
            let mut new_order = order.clone();
            new_order.quantity = remaining_qty;
            self.order_book.add_order(new_order);
        }

        let duration = start_time.elapsed();
        self.metrics.set_processing_time(duration);

        if !trades.is_empty() {
            info!("Executed {} trades for order {}", trades.len(), order.id);
        }
    }

    async fn process_cancellation(&self, _symbol: &str, order_id: u64) {
        if self.order_book.cancel_order(order_id) {
            log::info!("Cancelled order {}", order_id);
        } else {
            log::warn!("Failed to cancel order {}", order_id);
        }
    }

    fn match_limit_order(&self, order: &Order) -> Vec<Trade> {
        match order.side {
            OrderSide::Buy => self.match_buy_order(order, |ask_price| ask_price <= order.price),
            OrderSide::Sell => self.match_sell_order(order, |bid_price| bid_price >= order.price),
        }
    }

    fn match_market_order(&self, order: &Order) -> Vec<Trade> {
        match order.side {
            OrderSide::Buy => self.match_buy_order(order, |_| true),
            OrderSide::Sell => self.match_sell_order(order, |_| true),
        }
    }

    fn match_buy_order<F>(&self, order: &Order, price_check: F) -> Vec<Trade>
    where
        F: Fn(Decimal) -> bool,
    {
        let mut trades = Vec::new();
        let mut remaining_qty = order.quantity;
        let symbol = &order.symbol;

        if let Some(mut asks) = self.order_book.asks.get_mut(symbol) {
            let mut prices_to_check: Vec<Decimal> = asks.keys().cloned().collect();
            prices_to_check.sort();

            for price in prices_to_check {
                if !price_check(price) {
                    break;
                }

                if let Some(orders_at_price) = asks.get_mut(&price) {
                    let mut filled_indices = Vec::new();
                    let mut _filled_qty = Decimal::ZERO;

                    for (idx, resting_order) in orders_at_price.iter_mut().enumerate() {
                        if remaining_qty <= Decimal::ZERO {
                            break;
                        }

                        let trade_qty = remaining_qty.min(resting_order.quantity);

                        trades.push(Trade {
                            id: chrono::Utc::now().timestamp_nanos_opt().unwrap() as u64,
                            symbol: symbol.clone(),
                            price,
                            quantity: trade_qty,
                            buyer_id: order.id,
                            seller_id: resting_order.id,
                            timestamp: chrono::Utc::now().timestamp(),
                        });

                        remaining_qty -= trade_qty;
                        resting_order.quantity -= trade_qty;
                        _filled_qty += trade_qty;

                        if resting_order.quantity <= Decimal::new(1, 3) {
                            filled_indices.push(idx);
                            self.order_book.order_index.remove(&resting_order.id);
                        }
                    }

                    for idx in filled_indices.iter().rev() {
                        orders_at_price.remove(*idx);
                    }

                    if orders_at_price.is_empty() {
                        asks.remove(&price);
                    }
                }

                if remaining_qty <= Decimal::new(1, 3) {
                    break;
                }
            }
        }

        trades
    }

    fn match_sell_order<F>(&self, order: &Order, price_check: F) -> Vec<Trade>
    where
        F: Fn(Decimal) -> bool,
    {
        let mut trades = Vec::new();
        let mut remaining_qty = order.quantity;
        let symbol = &order.symbol;

        if let Some(mut bids) = self.order_book.bids.get_mut(symbol) {
            let mut prices_to_check: Vec<Decimal> = bids.keys().map(|k| k.0).collect();
            prices_to_check.sort_by(|a, b| b.cmp(a));

            for price in prices_to_check {
                if !price_check(price) {
                    break;
                }

                let price_key = Reverse(price);
                if let Some(orders_at_price) = bids.get_mut(&price_key) {
                    let mut filled_indices = Vec::new();
                    let mut _filled_qty = Decimal::ZERO;

                    for (idx, resting_order) in orders_at_price.iter_mut().enumerate() {
                        if remaining_qty <= Decimal::ZERO {
                            break;
                        }

                        let trade_qty = remaining_qty.min(resting_order.quantity);

                        trades.push(Trade {
                            id: chrono::Utc::now().timestamp_nanos_opt().unwrap() as u64,
                            symbol: symbol.clone(),
                            price,
                            quantity: trade_qty,
                            buyer_id: resting_order.id,
                            seller_id: order.id,
                            timestamp: chrono::Utc::now().timestamp(),
                        });

                        remaining_qty -= trade_qty;
                        resting_order.quantity -= trade_qty;
                        _filled_qty += trade_qty;

                        if resting_order.quantity <= Decimal::new(1, 3) {
                            filled_indices.push(idx);
                            self.order_book.order_index.remove(&resting_order.id);
                        }
                    }

                    for idx in filled_indices.iter().rev() {
                        orders_at_price.remove(*idx);
                    }

                    if orders_at_price.is_empty() {
                        bids.remove(&price_key);
                    }
                }

                if remaining_qty <= Decimal::new(1, 3) {
                    break;
                }
            }
        }

        trades
    }

    #[allow(dead_code)]
    pub async fn start_reporting(self: Arc<Self>, interval_secs: u64) {
        let engine = Arc::clone(&self);
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                // Existing position reporting
                engine.report_positions();

                // Additional detailed metrics
                let order_book = engine.order_book.clone();
                let (bid_depth, ask_depth) = order_book.get_order_book_depth("AAPL");
                log::info!("Detailed Trading Metrics:");
                log::info!(
                    "Order Book Depth - Bids: {}, Asks: {}",
                    bid_depth,
                    ask_depth
                );
                log::info!(
                    "Total Order Operations: {}",
                    order_book.get_operation_count()
                );

                // Potential risk analysis
                let risk_analysis = engine.risk_manager.analyze_portfolio_risk();
                for (symbol, metrics) in risk_analysis {
                    log::info!(
                        "Risk Analysis for {}: Position: {}, Realized PnL: {}, Limit Utilization: {}%",
                        symbol,
                        metrics.current_position(),
                        metrics.realized_pnl(),
                        metrics.utilization()
                    );
                }
            }
        });
    }
}
