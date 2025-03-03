use crate::order_book::{Order, OrderSide, OrderType};
use chrono::{DateTime, Utc};
use reqwest::Error;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct AlphaVantageResponse {
    #[serde(rename = "Time Series (1min)")]
    time_series: HashMap<String, MinuteData>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct MinuteData {
    #[serde(rename = "1. open")]
    pub open: f64,
    #[serde(rename = "2. high")]
    pub high: f64,
    #[serde(rename = "3. low")]
    pub low: f64,
    #[serde(rename = "4. close")]
    pub close: f64,
    #[serde(rename = "5. volume")]
    pub volume: f64,
}

/// Fetches market data from AlphaVantage for the given symbol.
/// Returns a vector of (timestamp, MinuteData) pairs.
pub async fn fetch_market_data(symbol: &str) -> Result<Vec<(DateTime<Utc>, MinuteData)>, Error> {
    let api_key =
        std::env::var("ALPHA_VANTAGE_API_KEY").unwrap_or_else(|_| "4RMSF2E3473M9N5J".to_string());
    let url = format!(
        "https://www.alphavantage.co/query?function=TIME_SERIES_INTRADAY&symbol={}&interval=1min&apikey={}",
        symbol, api_key
    );

    log::info!("Fetching market data for {}", symbol);

    let response = reqwest::get(&url)
        .await?
        .json::<AlphaVantageResponse>()
        .await?;

    let mut data = Vec::new();
    for (timestamp, values) in response.time_series {
        if let Ok(dt) = DateTime::parse_from_str(&timestamp, "%Y-%m-%d %H:%M:%S") {
            data.push((dt.with_timezone(&Utc), values));
        } else {
            log::warn!("Failed to parse timestamp: {}", timestamp);
        }
    }

    log::info!("Fetched {} data points for {}", data.len(), symbol);
    Ok(data)
}

impl MinuteData {
    /// Converts a minute's market data into a pair of limit orders:
    /// one buy order (with price slightly lower than the close)
    /// and one sell order (with price slightly higher than the close).
    pub fn to_orders(&self, symbol: &str) -> Vec<Order> {
        let spread_pct = 0.001; // 0.1% spread
        let spread = self.close * spread_pct;
        let ts = Utc::now().timestamp();

        let quantity = (self.volume * 0.001).max(10.0); // 0.1% of volume, minimum 10

        vec![
            Order {
                id: (ts * 1000) as u64,
                symbol: symbol.to_string(),
                price: self.close - spread,
                quantity,
                order_type: OrderType::Limit,
                side: OrderSide::Buy,
                timestamp: ts,
            },
            Order {
                id: (ts * 1000 + 1) as u64,
                symbol: symbol.to_string(),
                price: self.close + spread,
                quantity,
                order_type: OrderType::Limit,
                side: OrderSide::Sell,
                timestamp: ts,
            },
        ]
    }

    /// Creates more realistic market-making orders based on the OHLC data
    pub fn to_market_making_orders(&self, symbol: &str, layers: usize) -> Vec<Order> {
        let mut orders = Vec::with_capacity(layers * 2);
        let base_ts = Utc::now().timestamp();

        // Calculate volatility-based spread
        let volatility = (self.high - self.low) / self.close;
        let base_spread_pct = volatility.max(0.001); // At least 0.1% spread

        // Base quantity scaled to volume
        let base_quantity = (self.volume * 0.0005).max(10.0);

        for i in 0..layers {
            let layer_multiplier = 1.0 + (i as f64 * 0.5); // Increase spread by 50% per layer
            let price_offset = self.close * base_spread_pct * layer_multiplier;

            // Reduce quantity for outer layers
            let quantity_scalar = 1.0 / layer_multiplier;
            let quantity = base_quantity * quantity_scalar;

            // Buy order (bid)
            orders.push(Order {
                id: (base_ts * 1000 + (i * 2) as i64) as u64,
                symbol: symbol.to_string(),
                price: self.close - price_offset,
                quantity,
                order_type: OrderType::Limit,
                side: OrderSide::Buy,
                timestamp: base_ts,
            });

            // Sell order (ask)
            orders.push(Order {
                id: (base_ts * 1000 + (i * 2 + 1) as i64) as u64,
                symbol: symbol.to_string(),
                price: self.close + price_offset,
                quantity,
                order_type: OrderType::Limit,
                side: OrderSide::Sell,
                timestamp: base_ts,
            });
        }

        orders
    }
}

// Error handling wrapper for market data operations
pub struct MarketDataManager {
    symbols: Vec<String>,
    cache: HashMap<String, Vec<(DateTime<Utc>, MinuteData)>>,
    last_update: HashMap<String, DateTime<Utc>>,
}

impl MarketDataManager {
    pub fn new(symbols: Vec<String>) -> Self {
        MarketDataManager {
            symbols,
            cache: HashMap::new(),
            last_update: HashMap::new(),
        }
    }

    pub async fn update_data(
        &mut self,
    ) -> Vec<(String, Result<Vec<(DateTime<Utc>, MinuteData)>, Error>)> {
        let mut results = Vec::new();

        for symbol in &self.symbols {
            match fetch_market_data(symbol).await {
                Ok(data) => {
                    if !data.is_empty() {
                        let last_timestamp = data[0].0;
                        let cloned_data = data.clone();
                        self.cache.insert(symbol.clone(), cloned_data);
                        self.last_update.insert(symbol.clone(), last_timestamp);
                        results.push((symbol.clone(), Ok(data)));
                    } else {
                        log::warn!("Received empty data for {}", symbol);
                    }
                }
                Err(e) => {
                    log::error!("Failed to fetch data for {}: {}", symbol, e);
                    results.push((symbol.clone(), Err(e)));
                }
            }
        }

        results
    }

    pub fn get_cached_data(&self, symbol: &str) -> Option<&Vec<(DateTime<Utc>, MinuteData)>> {
        self.cache.get(symbol)
    }

    pub fn get_last_update(&self, symbol: &str) -> Option<DateTime<Utc>> {
        self.last_update.get(symbol).copied()
    }
}
