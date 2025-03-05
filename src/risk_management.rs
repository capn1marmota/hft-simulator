use crate::order_book::{Order, OrderSide};
use dashmap::DashMap;
use parking_lot::RwLock;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Mutex;

// Enhanced AtomicDecimal implementation
#[derive(Debug)]
pub struct AtomicDecimal {
    value: Mutex<Decimal>,
}

impl AtomicDecimal {
    pub fn new(initial_value: Decimal) -> Self {
        AtomicDecimal {
            value: Mutex::new(initial_value),
        }
    }

    pub fn get(&self) -> Decimal {
        *self.value.lock().unwrap()
    }

    pub fn set(&self, new_value: Decimal) {
        *self.value.lock().unwrap() = new_value;
    }

    pub fn add(&mut self, delta: Decimal) {
        let mut value = self.value.lock().unwrap();
        *value += delta;
    }

    pub fn is_sign_positive(&self) -> bool {
        self.get() > Decimal::ZERO
    }

    pub fn abs(&self) -> Decimal {
        self.get().abs()
    }

    pub fn try_increment(&self, delta: Decimal) -> bool {
        let mut value = self.value.lock().unwrap();
        if *value + delta >= Decimal::ZERO {
            *value += delta;
            true
        } else {
            false
        }
    }

    pub fn compare_and_swap(&self, expected: Decimal, new_value: Decimal) -> bool {
        let mut value = self.value.lock().unwrap();
        if *value == expected {
            *value = new_value;
            true
        } else {
            false
        }
    }
}

impl Clone for AtomicDecimal {
    fn clone(&self) -> Self {
        AtomicDecimal {
            value: Mutex::new(self.get()),
        }
    }
}

impl std::fmt::Display for AtomicDecimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}

impl std::ops::Mul<Decimal> for &AtomicDecimal {
    type Output = Decimal;

    fn mul(self, rhs: Decimal) -> Self::Output {
        self.get() * rhs
    }
}

// Struct to represent portfolio risk metrics
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RiskMetrics {
    current_position: Decimal,
    realized_pnl: Decimal,
    position_limit: Decimal,
    utilization: Decimal,
}

pub struct RiskManager {
    max_order_size: RwLock<Decimal>,
    position_limits: DashMap<String, Decimal>,
    current_positions: DashMap<String, AtomicDecimal>,
    realized_pnl: DashMap<String, AtomicDecimal>,
    avg_entry_prices: DashMap<String, Decimal>,
}

#[allow(dead_code)]
impl RiskMetrics {
    pub fn new(
        current_position: Decimal,
        realized_pnl: Decimal,
        position_limit: Decimal,
        utilization: Decimal,
    ) -> Self {
        Self {
            current_position,
            realized_pnl,
            position_limit,
            utilization,
        }
    }

    // Getter methods
    pub fn current_position(&self) -> Decimal {
        self.current_position
    }

    pub fn realized_pnl(&self) -> Decimal {
        self.realized_pnl
    }

    pub fn position_limit(&self) -> Decimal {
        self.position_limit
    }

    pub fn utilization(&self) -> Decimal {
        self.utilization
    }
}

impl RiskManager {
    pub fn new(max_order_size: Decimal) -> Self {
        RiskManager {
            max_order_size: RwLock::new(max_order_size),
            position_limits: DashMap::new(),
            current_positions: DashMap::new(),
            realized_pnl: DashMap::new(),
            avg_entry_prices: DashMap::new(),
        }
    }

    pub fn analyze_portfolio_risk(&self) -> HashMap<String, RiskMetrics> {
        let mut risk_metrics = HashMap::new();

        for entry in self.current_positions.iter() {
            let symbol = entry.key().clone();
            let current_position = entry.value().get();

            let realized_pnl = self
                .realized_pnl
                .get(&symbol)
                .map(|p| p.get())
                .unwrap_or(Decimal::ZERO);

            let position_limit = self
                .position_limits
                .get(&symbol)
                .map(|limit| *limit)
                .unwrap_or(Decimal::ZERO);

            risk_metrics.insert(
                symbol,
                RiskMetrics {
                    current_position,
                    realized_pnl,
                    position_limit,
                    utilization: if position_limit > Decimal::ZERO {
                        (current_position.abs() / position_limit * Decimal::from(100))
                            .min(Decimal::from(100))
                    } else {
                        Decimal::ZERO
                    },
                },
            );
        }

        risk_metrics
    }

    pub fn set_position_limit(&self, symbol: &str, limit: Decimal) {
        self.position_limits.insert(symbol.to_string(), limit);
    }

    pub fn validate_order(&self, order: &Order) -> bool {
        let max_size = *self.max_order_size.read();

        if order.quantity > max_size {
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
            .map(|p| p.get())
            .unwrap_or(Decimal::ZERO);

        let new_position = current + delta;

        if let Some(limit) = self.position_limits.get(symbol) {
            if new_position.abs() > *limit {
                return false;
            }
        }

        true
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

        // Get or insert current position
        let mut position_entry = self
            .current_positions
            .entry(symbol.to_string())
            .or_insert_with(|| AtomicDecimal::new(Decimal::ZERO));

        // Add to the position
        position_entry.add(signed_quantity);

        // Update or insert average entry price
        self.avg_entry_prices
            .entry(symbol.to_string())
            .and_modify(|avg_price| {
                let current_position = position_entry.get();
                if current_position != Decimal::ZERO {
                    *avg_price = (((*avg_price) * current_position.abs())
                        + (price * quantity.abs()))
                        / current_position.abs();
                }
            })
            .or_insert(price);

        // Ensure realized PnL entry exists
        self.realized_pnl
            .entry(symbol.to_string())
            .or_insert_with(|| AtomicDecimal::new(Decimal::ZERO));
    }

    pub fn report_positions(&self, get_price: impl Fn(&str) -> Option<Decimal>) {
        for entry in self.current_positions.iter() {
            let symbol = entry.key();
            let position = entry.value().get();

            let realized = self
                .realized_pnl
                .get(symbol)
                .map(|v| v.get())
                .unwrap_or(Decimal::ZERO);

            let avg_price = self
                .avg_entry_prices
                .get(symbol)
                .map(|r| *r)
                .unwrap_or(Decimal::ZERO);

            let unrealized = get_price(symbol)
                .map(|mp| {
                    if position > Decimal::ZERO {
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
