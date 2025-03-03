use crate::market_data::MinuteData;
use dashmap::DashMap;
use ordered_float::OrderedFloat;
use std::cmp::Reverse;
use std::collections::btree_map::Entry as BTreeEntry;
use std::collections::BTreeMap;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum OrderType {
    Market,
    Limit,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Order {
    pub id: u64,
    pub symbol: String,
    pub price: f64,
    pub quantity: f64,
    pub order_type: OrderType,
    pub side: OrderSide,
    pub timestamp: i64,
}

pub struct OrderBook {
    pub bids: DashMap<String, BTreeMap<Reverse<OrderedFloat<f64>>, Vec<Order>>>,
    pub asks: DashMap<String, BTreeMap<OrderedFloat<f64>, Vec<Order>>>,
    pub order_index: DashMap<u64, (String, OrderedFloat<f64>, OrderSide)>,
}

impl OrderBook {
    pub fn new() -> Self {
        OrderBook {
            bids: DashMap::new(),
            asks: DashMap::new(),
            order_index: DashMap::new(),
        }
    }

    pub fn add_order(&self, order: Order) {
        // Only limit orders can be added to the book
        if order.order_type != OrderType::Limit {
            log::warn!(
                "Unsupported order type for OrderBook: {:?}",
                order.order_type
            );
            return;
        }

        // Validate price and quantity
        if order.price <= 0.0 || order.quantity <= 0.0 {
            log::warn!(
                "Invalid order: price {} or quantity {} <= 0 for order ID {}",
                order.price,
                order.quantity,
                order.id
            );
            return;
        }

        // Add to index first
        log::debug!(
            "Adding order ID {} to index for symbol {}",
            order.id,
            order.symbol
        );
        self.order_index.insert(
            order.id,
            (
                order.symbol.clone(),
                OrderedFloat(order.price),
                order.side.clone(),
            ),
        );

        match order.side {
            OrderSide::Buy => {
                log::debug!("Adding buy order ID {} at price {}", order.id, order.price);
                let price_key = Reverse(OrderedFloat(order.price));
                let mut bids = match self.bids.get_mut(&order.symbol) {
                    Some(bids) => bids,
                    None => {
                        self.bids.insert(order.symbol.clone(), BTreeMap::new());
                        self.bids.get_mut(&order.symbol).unwrap()
                    }
                };
                bids.entry(price_key).or_insert_with(Vec::new).push(order);
            }
            OrderSide::Sell => {
                log::debug!("Adding sell order ID {} at price {}", order.id, order.price);
                let price_key = OrderedFloat(order.price);
                let mut asks = match self.asks.get_mut(&order.symbol) {
                    Some(asks) => asks,
                    None => {
                        self.asks.insert(order.symbol.clone(), BTreeMap::new());
                        self.asks.get_mut(&order.symbol).unwrap()
                    }
                };
                asks.entry(price_key).or_insert_with(Vec::new).push(order);
            }
        }
    }

    pub fn cancel_order(&self, order_id: u64) -> bool {
        log::debug!("Attempting to cancel order ID {}", order_id);

        if let Some((_, (sym, price, side))) = self.order_index.remove(&order_id) {
            log::debug!(
                "Found order ID {} in index: symbol={}, price={}, side={:?}",
                order_id,
                sym,
                price,
                side
            );
            match side {
                OrderSide::Buy => {
                    if let Some(mut bids) = self.bids.get_mut(&sym) {
                        let price_key = Reverse(price);
                        match bids.entry(price_key) {
                            BTreeEntry::Occupied(mut price_entry) => {
                                let orders = price_entry.get_mut();
                                let len_before = orders.len();
                                orders.retain(|o| o.id != order_id);
                                let cancelled = len_before != orders.len();
                                if orders.is_empty() {
                                    log::debug!(
                                        "Removing empty price level {} for symbol {} (bids)",
                                        price,
                                        sym
                                    );
                                    price_entry.remove_entry();
                                }
                                if !cancelled {
                                    log::debug!(
                                        "Order ID {} not found at price {} in bids for symbol {}",
                                        order_id,
                                        price,
                                        sym
                                    );
                                }
                                cancelled
                            }
                            BTreeEntry::Vacant(_) => {
                                log::debug!(
                                    "Price level {} not found in bids for symbol {}",
                                    price,
                                    sym
                                );
                                false
                            }
                        }
                    } else {
                        log::debug!("Symbol {} not found in bids", sym);
                        false
                    }
                }
                OrderSide::Sell => {
                    if let Some(mut asks) = self.asks.get_mut(&sym) {
                        let price_key = price;
                        match asks.entry(price_key) {
                            BTreeEntry::Occupied(mut price_entry) => {
                                let orders = price_entry.get_mut();
                                let len_before = orders.len();
                                orders.retain(|o| o.id != order_id);
                                let cancelled = len_before != orders.len();
                                if orders.is_empty() {
                                    log::debug!(
                                        "Removing empty price level {} for symbol {} (asks)",
                                        price,
                                        sym
                                    );
                                    price_entry.remove_entry();
                                }
                                if !cancelled {
                                    log::debug!(
                                        "Order ID {} not found at price {} in asks for symbol {}",
                                        order_id,
                                        price,
                                        sym
                                    );
                                }
                                cancelled
                            }
                            BTreeEntry::Vacant(_) => {
                                log::debug!(
                                    "Price level {} not found in asks for symbol {}",
                                    price,
                                    sym
                                );
                                false
                            }
                        }
                    } else {
                        log::debug!("Symbol {} not found in asks", sym);
                        false
                    }
                }
            }
        } else {
            log::debug!("Order ID {} not found in order_index", order_id);
            false
        }
    }

    pub fn get_best_bid(&self, symbol: &str) -> Option<f64> {
        self.bids.get(symbol).and_then(|bids_ref| {
            bids_ref
                .last_key_value()
                .map(|(price, _)| price.0.into_inner())
        })
    }

    pub fn get_best_ask(&self, symbol: &str) -> Option<f64> {
        self.asks.get(symbol).and_then(|asks_ref| {
            asks_ref
                .first_key_value()
                .map(|(price, _)| price.into_inner())
        })
    }

    pub fn get_mid_price(&self, symbol: &str) -> Option<f64> {
        self.get_best_bid(symbol)
            .zip(self.get_best_ask(symbol))
            .map(|(bid, ask)| (bid + ask) / 2.0)
    }

    pub fn update_from_market_data(&self, symbol: &str, data: &MinuteData) {
        #[allow(dead_code)]
        let orders = data.to_orders(symbol);
        for order in orders {
            self.add_order(order);
        }
    }
}
