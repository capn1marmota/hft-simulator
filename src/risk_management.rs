use crate::order_book::{Order, OrderSide};
use dashmap::DashMap;

pub struct RiskManager {
    max_order_size: f64,
    position_limits: DashMap<String, f64>,
    current_positions: DashMap<String, f64>,
    realized_pnl: DashMap<String, f64>,
    avg_entry_prices: DashMap<String, f64>,
}

impl RiskManager {
    pub fn new(max_order_size: f64) -> Self {
        RiskManager {
            max_order_size,
            position_limits: DashMap::new(),
            current_positions: DashMap::new(),
            realized_pnl: DashMap::new(),
            avg_entry_prices: DashMap::new(),
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

    pub fn set_position_limit(&self, symbol: &str, limit: f64) {
        self.position_limits.insert(symbol.to_string(), limit);
    }

    pub fn get_position(&self, symbol: &str) -> f64 {
        self.current_positions
            .get(symbol)
            .map(|p| *p)
            .unwrap_or(0.0)
    }
    #[allow(dead_code)]
    pub fn get_realized_pnl(&self, symbol: &str) -> f64 {
        self.realized_pnl
            .get(symbol)
            .map(|p| *p)
            .unwrap_or(0.0)
    }

    #[allow(dead_code)]
    pub fn get_avg_price(&self, symbol: &str) -> f64 {
        self.avg_entry_prices
            .get(symbol)
            .map(|p| *p)
            .unwrap_or(0.0)
    }

    pub fn record_transaction(&self, symbol: &str, price: f64, quantity: f64, side: OrderSide) {
        if quantity <= 0.0 {
            return;
        }

        let signed_quantity = match side {
            OrderSide::Buy => quantity,
            OrderSide::Sell => -quantity,
        };

        // Update position
        let mut position_entry = self.current_positions.entry(symbol.to_string()).or_insert(0.0);
        let old_position = *position_entry;
        *position_entry += signed_quantity;
        let new_position = *position_entry;

        // Update average price and PnL
        let mut avg_price_entry = self.avg_entry_prices.entry(symbol.to_string()).or_insert(0.0);
        let mut realized_entry = self.realized_pnl.entry(symbol.to_string()).or_insert(0.0);

        let old_avg_price = *avg_price_entry;
        let mut new_avg_price = old_avg_price;
        let mut realized_pnl = 0.0;

        // Handle position changes
        if old_position == 0.0 {
            // New position
            new_avg_price = price;
        } else if old_position.signum() != new_position.signum() {
            // Position flipped (long <-> short)
            realized_pnl = (price - old_avg_price) * old_position.abs();
            new_avg_price = price;
        } else if old_position.abs() < new_position.abs() {
            // Adding to position
            let total_cost = (old_position.abs() * old_avg_price) + (quantity * price);
            new_avg_price = total_cost / new_position.abs();
        }

        // Update stored values
        *avg_price_entry = new_avg_price;
        *realized_entry += realized_pnl;

        log::info!(
            "Transaction: {} {} x {} @ {:.4} (Pos: {:.2} -> {:.2}, Avg: {:.2}, PnL: {:.2})",
            match side {
                OrderSide::Buy => "Buy",
                OrderSide::Sell => "Sell",
            },
            quantity,
            symbol,
            price,
            old_position,
            new_position,
            new_avg_price,
            realized_pnl
        );
    }
}