use crate::market_data::MinuteData;
use dashmap::DashMap;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::cmp::Reverse;
use std::collections::{btree_map::Entry, BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum OrderType {
    Market,
    Limit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[allow(dead_code)]
struct AtomicOrderQueue {
    orders: Mutex<VecDeque<Order>>,
    total_quantity: AtomicU64,
}
#[allow(dead_code)]
impl AtomicOrderQueue {
    fn new() -> Self {
        Self {
            orders: Mutex::new(VecDeque::new()),
            total_quantity: AtomicU64::new(0),
        }
    }

    #[allow(dead_code)]
    fn add_order(&self, order: Order) {
        let quantity_u64 = order.quantity.to_u64().unwrap_or(0);

        // Atomic quantity update
        self.total_quantity
            .fetch_add(quantity_u64, Ordering::Relaxed);

        // Thread-safe order insertion
        let mut orders = self.orders.lock().unwrap();
        orders.push_back(order);
    }

    #[allow(dead_code)]
    fn remove_order(&self, order_id: u64) -> Option<Order> {
        let mut orders = self.orders.lock().unwrap();
        if let Some(idx) = orders.iter().position(|o| o.id == order_id) {
            let order = orders.remove(idx).unwrap();

            // Atomic quantity reduction
            let quantity_u64 = order.quantity.to_u64().unwrap_or(0);
            self.total_quantity
                .fetch_sub(quantity_u64, Ordering::Relaxed);

            Some(order)
        } else {
            None
        }
    }
}
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Order {
    pub id: u64,
    pub symbol: String,
    pub price: Decimal,
    pub quantity: Decimal,
    pub order_type: OrderType,
    pub side: OrderSide,
    pub timestamp: i64,
}

pub struct OrderBook {
    pub bids: DashMap<String, BTreeMap<Reverse<Decimal>, Vec<Order>>>,
    pub asks: DashMap<String, BTreeMap<Decimal, Vec<Order>>>,
    pub order_index: DashMap<u64, (String, Decimal, OrderSide)>,

    // Performance tracking
    order_operations: AtomicUsize,
}

#[allow(dead_code)]
impl OrderBook {
    pub fn new() -> Self {
        OrderBook {
            bids: DashMap::new(),
            asks: DashMap::new(),
            order_index: DashMap::new(),
            order_operations: AtomicUsize::new(0),
        }
    }

    #[allow(dead_code)]
    pub fn load_historical_market_data(
        &self,
        historical_data: &[MinuteData],
        symbol: &str,
        tick_size: Decimal,
    ) {
        for data in historical_data {
            self.update_from_market_data(symbol, data, tick_size);
        }
    }

    pub fn add_order(&self, order: Order) {
        // Increment operation counter
        self.order_operations.fetch_add(1, Ordering::Relaxed);

        if order.order_type != OrderType::Limit {
            log::warn!(
                "Unsupported order type for OrderBook: {:?}",
                order.order_type
            );
            return;
        }

        if order.price <= Decimal::ZERO || order.quantity <= Decimal::ZERO {
            log::warn!(
                "Invalid order: price {} or quantity {} <= 0 for order ID {}",
                order.price,
                order.quantity,
                order.id
            );
            return;
        }

        // Store order index
        self.order_index.insert(
            order.id,
            (order.symbol.clone(), order.price, order.side.clone()),
        );

        match order.side {
            OrderSide::Buy => {
                let price_key = Reverse(order.price);
                let mut bids = self
                    .bids
                    .entry(order.symbol.clone())
                    .or_insert_with(BTreeMap::new);
                bids.entry(price_key).or_insert_with(Vec::new).push(order);
            }
            OrderSide::Sell => {
                let price_key = order.price;
                let mut asks = self
                    .asks
                    .entry(order.symbol.clone())
                    .or_insert_with(BTreeMap::new);
                asks.entry(price_key).or_insert_with(Vec::new).push(order);
            }
        }
    }

    pub fn cancel_order(&self, order_id: u64) -> bool {
        // Increment operation counter
        self.order_operations.fetch_add(1, Ordering::Relaxed);

        if let Some((_, (sym, price, side))) = self.order_index.remove(&order_id) {
            match side {
                OrderSide::Buy => {
                    if let Some(mut bids) = self.bids.get_mut(&sym) {
                        let price_key = Reverse(price);
                        if let Entry::Occupied(mut price_entry) = bids.entry(price_key) {
                            let (modified, is_empty) = {
                                let orders = price_entry.get_mut();
                                let len_before = orders.len();
                                orders.retain(|o| o.id != order_id);
                                (len_before != orders.len(), orders.is_empty())
                            };

                            if is_empty {
                                price_entry.remove_entry();
                            }
                            return modified;
                        }
                    }
                }
                OrderSide::Sell => {
                    if let Some(mut asks) = self.asks.get_mut(&sym) {
                        let price_key = price;
                        if let Entry::Occupied(mut price_entry) = asks.entry(price_key) {
                            let (modified, is_empty) = {
                                let orders = price_entry.get_mut();
                                let len_before = orders.len();
                                orders.retain(|o| o.id != order_id);
                                (len_before != orders.len(), orders.is_empty())
                            };

                            if is_empty {
                                price_entry.remove_entry();
                            }
                            return modified;
                        }
                    }
                }
            }
        }
        false
    }

    // Existing methods from original implementation
    pub fn get_best_bid(&self, symbol: &str) -> Option<Decimal> {
        self.bids
            .get(symbol)
            .and_then(|bids| bids.last_key_value().map(|(price, _)| price.0))
    }

    pub fn get_best_ask(&self, symbol: &str) -> Option<Decimal> {
        self.asks
            .get(symbol)
            .and_then(|asks| asks.first_key_value().map(|(price, _)| *price))
    }

    pub fn get_mid_price(&self, symbol: &str) -> Option<Decimal> {
        match (self.get_best_bid(symbol), self.get_best_ask(symbol)) {
            (Some(bid), Some(ask)) => Some((bid + ask) / Decimal::from(2)),
            _ => None,
        }
    }

    pub fn get_order_book_depth(&self, symbol: &str) -> (usize, usize) {
        let bid_depth = self
            .bids
            .get(symbol)
            .map(|bids| bids.values().map(|orders| orders.len()).sum())
            .unwrap_or(0);

        let ask_depth = self
            .asks
            .get(symbol)
            .map(|asks| asks.values().map(|orders| orders.len()).sum())
            .unwrap_or(0);

        (bid_depth, ask_depth)
    }

    // New performance monitoring method
    pub fn get_operation_count(&self) -> usize {
        self.order_operations.load(Ordering::Relaxed)
    }

    #[allow(dead_code)]
    pub fn update_from_market_data(&self, symbol: &str, data: &MinuteData, tick_size: Decimal) {
        let orders = data.to_orders(symbol, tick_size);
        for order in orders {
            self.add_order(order);
        }
    }
}
