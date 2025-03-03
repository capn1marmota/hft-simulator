use dashmap::DashMap;
use ordered_float::OrderedFloat;
use std::collections::BTreeMap;
use std::cmp::Reverse;
use crate::market_data::MinuteData;

#[allow(dead_code)]  // Market orders planned for phase 2
#[derive(Debug, Clone, PartialEq)]
pub enum OrderType {
    Market,
    Limit,
}

#[allow(dead_code)]  // Full order side support needed
#[derive(Debug, Clone, PartialEq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[allow(dead_code)]  // Fields needed for auditing/analytics
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

// Single OrderBook struct definition
pub struct OrderBook {
    pub bids: DashMap<String, BTreeMap<Reverse<OrderedFloat<f64>>, Vec<Order>>>, // Bids: highest price first
    pub asks: DashMap<String, BTreeMap<OrderedFloat<f64>, Vec<Order>>>,          // Asks: lowest price first
}

impl OrderBook {
    pub fn new() -> Self {
        OrderBook {
            bids: DashMap::new(),
            asks: DashMap::new(),
        }
    }
    
    pub fn add_order(&self, order: Order) {
        // Only limit orders can be added to the book
        if order.order_type != OrderType::Limit {
            panic!("Only limit orders can be added to the order book");
        }
    
        match order.side {
            OrderSide::Buy => {
                let price_key = Reverse(OrderedFloat(order.price));
                self.bids
                    .entry(order.symbol.clone())
                    .or_insert_with(BTreeMap::new)
                    .entry(price_key)
                    .or_insert_with(Vec::new)
                    .push(order);
            }
            OrderSide::Sell => {
                let price_key = OrderedFloat(order.price);
                self.asks
                    .entry(order.symbol.clone())
                    .or_insert_with(BTreeMap::new)
                    .entry(price_key)
                    .or_insert_with(Vec::new)
                    .push(order);
            }
        }
    }

    pub fn get_best_bid(&self, symbol: &str) -> Option<f64> {
        self.bids.get(symbol)
            .and_then(|bids_ref| {
                // The last key in a BTreeMap of bids (using Reverse keys) is the highest bid
                bids_ref.last_key_value().map(|(price, _)| price.0.into_inner())
            })
    }
    
    pub fn get_best_ask(&self, symbol: &str) -> Option<f64> {
        self.asks.get(symbol)
            .and_then(|asks_ref| {
                // The first key in a BTreeMap of asks is the lowest ask
                asks_ref.first_key_value().map(|(price, _)| price.into_inner())
            })
    }

    pub fn get_mid_price(&self, symbol: &str) -> Option<f64> {
        self.get_best_bid(symbol)
            .zip(self.get_best_ask(symbol))
            .map(|(bid, ask)| (bid + ask) / 2.0)
    }

    pub fn update_from_market_data(&self, symbol: &str, data: &MinuteData) {
        #[allow(dead_code)] // Temporary until full integration
        let orders = data.to_orders(symbol);
        for order in orders {
            self.add_order(order);
        }
    }

    #[allow(unused_variables)] // For len_before
    pub fn cancel_order(&self, symbol: &str, order_id: u64) -> bool {
        let mut cancelled = false;
        
        // Check bids
        self.bids.get_mut(symbol).map(|mut bids| {
            bids.retain(|_, orders| {
                let original_len = orders.len();
                orders.retain(|o| o.id != order_id);
                cancelled |= orders.len() < original_len;
                !orders.is_empty()
            });
        });
        
        // Check asks if not found in bids
        if !cancelled {
            self.asks.get_mut(symbol).map(|mut asks| {
                asks.retain(|_, orders| {
                    let original_len = orders.len();
                    orders.retain(|o| o.id != order_id);
                    cancelled |= orders.len() < original_len;
                    !orders.is_empty()
                });
            });
        }

        cancelled
    }
}
