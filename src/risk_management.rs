use crate::order_book::{Order, OrderSide};
use dashmap::DashMap;

pub struct RiskManager {
    max_order_size: f64,
    position_limits: DashMap<String, f64>,
    current_positions: DashMap<String, f64>,
}

impl RiskManager {
    pub fn new(max_order_size: f64) -> Self {
        RiskManager {
            max_order_size,
            position_limits: DashMap::new(),
            current_positions: DashMap::new(),
        }
    }

    pub fn validate_order(&self, order: &Order) -> bool {
        // Check maximum order size
        if order.quantity > self.max_order_size {
            return false;
        }

        let symbol = &order.symbol;
        let delta = match order.side {
            OrderSide::Buy => order.quantity,
            OrderSide::Sell => -order.quantity,
        };

        // Calculate potential new position
        let current = self.current_positions
            .get(symbol)
            .map(|p| *p)
            .unwrap_or(0.0);
        
        let new_position = current + delta;

        // Check position limits
        if let Some(limit) = self.position_limits.get(symbol) {
            if new_position.abs() > *limit {
                return false;
            }
        }

        true
    }

    pub fn update_position(&self, order: &Order) {
        let symbol = order.symbol.clone();
        let delta = match order.side {
            OrderSide::Buy => order.quantity,
            OrderSide::Sell => -order.quantity,
        };

        self.current_positions
            .entry(symbol)
            .and_modify(|pos| *pos += delta)
            .or_insert(delta);
    }

    pub fn set_position_limit(&self, symbol: &str, limit: f64) {
        self.position_limits.insert(symbol.to_string(), limit);
    }

    pub fn get_position(&self, symbol: &str) -> f64 {
        self.current_positions
            .get(symbol)
            .map(|p| *p)
            .unwrap_or(0.0)
    }
}