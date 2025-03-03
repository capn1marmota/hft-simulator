use crate::order_book::{Order, OrderSide};
use dashmap::DashMap;
use rust_decimal::prelude::Signed;
use rust_decimal::Decimal;

pub struct RiskManager {
    max_order_size: Decimal,
    position_limits: DashMap<String, Decimal>,
    current_positions: DashMap<String, Decimal>,
    realized_pnl: DashMap<String, Decimal>,
    avg_entry_prices: DashMap<String, Decimal>,
}

impl RiskManager {
    pub fn new(max_order_size: Decimal) -> Self {
        RiskManager {
            max_order_size,
            position_limits: DashMap::new(),
            current_positions: DashMap::new(),
            realized_pnl: DashMap::new(),
            avg_entry_prices: DashMap::new(),
        }
    }

    pub fn validate_order(&self, order: &Order) -> bool {
        if order.quantity > self.max_order_size {
            return false;
        }

        let symbol = &order.symbol;
        let delta = if order.side == OrderSide::Buy {
            order.quantity
        } else {
            -order.quantity
        };

        let current = self
            .current_positions
            .get(symbol)
            .map(|p| *p)
            .unwrap_or(Decimal::ZERO);
        let new_position = current + delta;

        if let Some(limit) = self.position_limits.get(symbol) {
            if new_position.abs() > *limit {
                return false;
            }
        }

        true
    }

    pub fn set_position_limit(&self, symbol: &str, limit: Decimal) {
        self.position_limits.insert(symbol.to_string(), limit);
    }

    #[allow(dead_code)]
    pub fn get_position(&self, symbol: &str) -> Decimal {
        self.current_positions
            .get(symbol)
            .map(|p| *p)
            .unwrap_or(Decimal::ZERO)
    }

    #[allow(dead_code)]
    pub fn get_realized_pnl(&self, symbol: &str) -> Decimal {
        self.realized_pnl
            .get(symbol)
            .map(|p| *p)
            .unwrap_or(Decimal::ZERO)
    }

    #[allow(dead_code)]
    pub fn get_avg_price(&self, symbol: &str) -> Decimal {
        self.avg_entry_prices
            .get(symbol)
            .map(|p| *p)
            .unwrap_or(Decimal::ZERO)
    }

    pub fn record_transaction(
        &self,
        symbol: &str,
        price: Decimal,
        quantity: Decimal,
        side: OrderSide,
    ) {
        if quantity <= Decimal::ZERO {
            return;
        }

        let signed_quantity = if side == OrderSide::Buy {
            quantity
        } else {
            -quantity
        };

        let mut position_entry = self
            .current_positions
            .entry(symbol.to_string())
            .or_insert(Decimal::ZERO);
        let old_position = *position_entry;
        *position_entry += signed_quantity;
        let new_position = *position_entry;

        let mut avg_price_entry = self
            .avg_entry_prices
            .entry(symbol.to_string())
            .or_insert(Decimal::ZERO);
        let mut realized_entry = self
            .realized_pnl
            .entry(symbol.to_string())
            .or_insert(Decimal::ZERO);

        let old_avg_price = *avg_price_entry;

        if old_position != Decimal::ZERO && old_position.signum() != new_position.signum() {
            let pnl = (price - old_avg_price) * old_position.abs();
            *realized_entry += pnl;
            *avg_price_entry = price;
        } else if new_position.abs() < old_position.abs() {
            let closed_quantity = (old_position.abs() - new_position.abs()).min(quantity);
            let pnl = if side == OrderSide::Sell {
                (price - old_avg_price) * closed_quantity
            } else {
                (old_avg_price - price) * closed_quantity
            };
            *realized_entry += pnl;
        } else if new_position != Decimal::ZERO {
            let total_quantity = old_position.abs() + quantity;
            *avg_price_entry =
                ((old_avg_price * old_position.abs()) + (price * quantity)) / total_quantity;
        } else {
            *avg_price_entry = Decimal::ZERO;
        }
    }

    pub fn report_positions(&self, get_price: impl Fn(&str) -> Option<Decimal>) {
        for entry in self.current_positions.iter() {
            let symbol = entry.key();
            let position = entry.value();
            let realized = self
                .realized_pnl
                .get(symbol)
                .map(|v| *v)
                .unwrap_or(Decimal::ZERO);
            let avg_price = self
                .avg_entry_prices
                .get(symbol)
                .map(|v| *v)
                .unwrap_or(Decimal::ZERO);

            let unrealized = get_price(symbol)
                .map(|mp| {
                    if position.is_sign_positive() {
                        (mp - avg_price) * position
                    } else {
                        (avg_price - mp) * position.abs()
                    }
                })
                .unwrap_or(Decimal::ZERO);

            log::info!(
                "Position Report | {} | Size: {:.2} | Avg: {:.2} | Realized: {:.2} | Unrealized: {:.2}",
                symbol, position, avg_price, realized, unrealized
            );
        }
    }
}
