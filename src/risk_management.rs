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

    #[allow(dead_code)]  // Used for debugging/reporting
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

        // Update average price and realized PnL
        let mut avg_price_entry = self.avg_entry_prices.entry(symbol.to_string()).or_insert(0.0);
        let mut realized_entry = self.realized_pnl.entry(symbol.to_string()).or_insert(0.0);

        let old_avg_price = *avg_price_entry;

        if old_position != 0.0 && new_position.signum() != old_position.signum() {
            // Position reversal: fully close old position and start a new one.
            let pnl = (price - old_avg_price) * old_position.abs();
            *realized_entry += pnl;
            *avg_price_entry = price; // Reset average price to current price for the new position
        } else if new_position.abs() < old_position.abs() {
            // Partial closure: only part of the position is closed; average price remains unchanged.
            let closed_quantity = (old_position.abs() - new_position.abs()).min(quantity);
            let pnl = match side {
                OrderSide::Sell => (price - old_avg_price) * closed_quantity,
                OrderSide::Buy => (old_avg_price - price) * closed_quantity,
            };
            *realized_entry += pnl;
            // Leave avg_price_entry unchanged for the remaining position.
        } else if new_position != 0.0 {
            // Position increase (or a consistent position): update weighted average price.
            let total_quantity = old_position.abs() + quantity;
            *avg_price_entry = ((old_avg_price * old_position.abs()) + (price * quantity)) / total_quantity;
        } else {
            // Position fully closed: reset average price.
            *avg_price_entry = 0.0;
        }

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
            *avg_price_entry,
            *realized_entry
        );
    }

    #[allow(dead_code)]
    pub fn report_positions(&self, get_price: impl Fn(&str) -> Option<f64>) {
        for entry in self.current_positions.iter() {
            let symbol = entry.key();
            let position = entry.value();
            let realized = self.realized_pnl.get(symbol).map(|v| *v).unwrap_or(0.0);
            let avg_price = self.avg_entry_prices.get(symbol).map(|v| *v).unwrap_or(0.0);
            
            let unrealized = get_price(symbol)
                .map(|mp| match position.signum() {
                    1.0 => (mp - avg_price) * position,
                    -1.0 => (avg_price - mp) * position.abs(),
                    _ => 0.0
                })
                .unwrap_or(0.0);

            log::info!(
                "Position Report | {} | Size: {:.2} | Avg: {:.2} | Realized: {:.2} | Unrealized: {:.2}",
                symbol, position, avg_price, realized, unrealized
            );
        }
    }
}