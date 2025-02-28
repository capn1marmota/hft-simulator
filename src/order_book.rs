use dashmap::DashMap;
use ordered_float::OrderedFloat;
use std::collections::BTreeMap;
use std::cmp::Reverse;

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
        self.bids.get(symbol)?
            .first_key_value()
            .map(|(price, _)| price.0.into_inner())
    }
    
    pub fn get_best_ask(&self, symbol: &str) -> Option<f64> {
        self.asks.get(symbol)?
            .first_key_value()
            .map(|(price, _)| price.into_inner())
    }
}