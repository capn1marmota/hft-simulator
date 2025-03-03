use crate::order_book::{Order, OrderSide, OrderType};
use chrono::{DateTime, Utc};
use reqwest;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};
use std::{collections::HashMap, str::FromStr, time::Duration};

/// Custom error type for market data fetching.
#[derive(Debug)]
pub enum MarketDataError {
    /// Error when the API key is missing.
    MissingApiKey,
    /// Wrapper for errors returned by reqwest.
    Reqwest(reqwest::Error),
    /// Error when the API responds with a non-success status code.
    ApiError(String),
}

impl std::fmt::Display for MarketDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarketDataError::MissingApiKey => {
                write!(f, "ALPHA_VANTAGE_API_KEY environment variable not set")
            }
            MarketDataError::Reqwest(e) => write!(f, "Reqwest error: {}", e),
            MarketDataError::ApiError(msg) => write!(f, "API error: {}", msg),
        }
    }
}

impl std::error::Error for MarketDataError {}

impl From<reqwest::Error> for MarketDataError {
    fn from(e: reqwest::Error) -> Self {
        MarketDataError::Reqwest(e)
    }
}

/// Structure representing the Alpha Vantage API response.
#[derive(Debug, Deserialize)]
struct AlphaVantageResponse {
    #[serde(rename = "Time Series (1min)")]
    time_series: HashMap<String, MinuteData>,
}

/// Structure representing one minute of market data.
#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct MinuteData {
    #[serde(rename = "1. open", deserialize_with = "deserialize_decimal")]
    pub open: Decimal,
    #[serde(rename = "2. high", deserialize_with = "deserialize_decimal")]
    pub high: Decimal,
    #[serde(rename = "3. low", deserialize_with = "deserialize_decimal")]
    pub low: Decimal,
    #[serde(rename = "4. close", deserialize_with = "deserialize_decimal")]
    pub close: Decimal,
    #[serde(rename = "5. volume", deserialize_with = "deserialize_decimal")]
    pub volume: Decimal,
}

/// Custom deserializer to convert API string values into Decimal.
fn deserialize_decimal<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Decimal::from_str(&s).map_err(serde::de::Error::custom)
}

/// Fetch market data from Alpha Vantage for a given symbol.
///
/// This function retrieves 1-minute interval market data, converts the API’s string values
/// into `Decimal`, and returns a sorted vector (in descending order by timestamp) of tuples
/// containing the timestamp and corresponding market data.
///
/// The function will retry up to 3 times in case of transient network errors.
///
/// # Errors
///
/// Returns a `MarketDataError` if:
/// - The API key is missing.
/// - The HTTP request fails or the response is invalid.
/// - The API returns a non-success status code.
pub async fn fetch_market_data(
    symbol: &str,
) -> Result<Vec<(DateTime<Utc>, MinuteData)>, MarketDataError> {
    // Retrieve the API key from the environment.
    let api_key =
        std::env::var("ALPHA_VANTAGE_API_KEY").map_err(|_| MarketDataError::MissingApiKey)?;

    // Build the API URL.
    let url = format!(
        "https://www.alphavantage.co/query?function=TIME_SERIES_INTRADAY&symbol={}&interval=1min&apikey={}",
        symbol, api_key
    );

    // Retry logic: attempt up to 3 times for transient errors.
    let mut attempts = 3;
    let mut last_error: Option<reqwest::Error> = None;
    while attempts > 0 {
        match reqwest::get(&url).await {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    let body = response.text().await.unwrap_or_default();
                    return Err(MarketDataError::ApiError(format!(
                        "Status {}: {}",
                        status, body
                    )));
                }
                // Parse JSON response.
                let json_response = response.json::<AlphaVantageResponse>().await?;
                let mut data = Vec::new();
                // Convert string timestamps to DateTime<Utc> and collect the data.
                for (timestamp, values) in json_response.time_series {
                    if let Ok(dt) = DateTime::parse_from_str(&timestamp, "%Y-%m-%d %H:%M:%S") {
                        data.push((dt.with_timezone(&Utc), values));
                    }
                }
                // Sort data in descending order by timestamp (most recent first).
                data.sort_by(|a, b| b.0.cmp(&a.0));
                return Ok(data);
            }
            Err(e) => {
                last_error = Some(e);
                // Wait for 2 seconds before retrying.
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        attempts -= 1;
    }
    Err(MarketDataError::Reqwest(last_error.unwrap()))
}

impl MinuteData {
    /// Convert minute data into two limit orders (one buy and one sell) with a fixed spread.
    pub fn to_orders(&self, symbol: &str, tick_size: Decimal) -> Vec<Order> {
        // Define spread percentage (0.1%).
        let spread_pct = Decimal::new(1, 3);
        let spread = self.close * spread_pct;
        // Generate a unique timestamp for order IDs.
        let ts = Utc::now().timestamp_nanos_opt().unwrap_or_default() as u64;
        // Calculate a base order quantity with a minimum threshold.
        let base_quantity = (self.volume * Decimal::new(1, 3)).max(Decimal::new(10, 0));

        vec![
            Order {
                id: ts,
                symbol: symbol.to_string(),
                price: round_to_tick(self.close - spread, tick_size),
                quantity: base_quantity,
                order_type: OrderType::Limit,
                side: OrderSide::Buy,
                timestamp: ts as i64,
            },
            Order {
                id: ts + 1,
                symbol: symbol.to_string(),
                price: round_to_tick(self.close + spread, tick_size),
                quantity: base_quantity,
                order_type: OrderType::Limit,
                side: OrderSide::Sell,
                timestamp: ts as i64,
            },
        ]
    }

    /// Convert minute data into layered market making orders.
    ///
    /// This method creates multiple layers of buy and sell orders based on volatility.
    #[allow(dead_code)]
    pub fn to_market_making_orders(
        &self,
        symbol: &str,
        layers: usize,
        tick_size: Decimal,
    ) -> Vec<Order> {
        let mut orders = Vec::with_capacity(layers * 2);
        let ts = Utc::now().timestamp_nanos_opt().unwrap_or_default() as u64;
        // Calculate volatility-based spread.
        let volatility = (self.high - self.low) / self.close;
        let base_spread = volatility.max(Decimal::new(1, 3));
        // Calculate base quantity with a minimum threshold.
        let base_quantity = (self.volume * Decimal::new(5, 4)).max(Decimal::new(10, 0));

        for i in 0..layers {
            let layer_multiplier = Decimal::from(i as i64 + 1);
            let price_offset = self.close * base_spread * layer_multiplier;
            let quantity = base_quantity / layer_multiplier;

            orders.push(Order {
                id: ts + (i * 2) as u64,
                symbol: symbol.to_string(),
                price: round_to_tick(self.close - price_offset, tick_size),
                quantity,
                order_type: OrderType::Limit,
                side: OrderSide::Buy,
                timestamp: ts as i64,
            });

            orders.push(Order {
                id: ts + (i * 2 + 1) as u64,
                symbol: symbol.to_string(),
                price: round_to_tick(self.close + price_offset, tick_size),
                quantity,
                order_type: OrderType::Limit,
                side: OrderSide::Sell,
                timestamp: ts as i64,
            });
        }
        orders
    }
}

/// Rounds the provided price to the nearest multiple of the tick size.
fn round_to_tick(price: Decimal, tick_size: Decimal) -> Decimal {
    (price / tick_size).round() * tick_size
}

/// Manager for caching and updating market data.
#[allow(dead_code)]
pub struct MarketDataManager {
    cache: HashMap<String, Vec<(DateTime<Utc>, MinuteData)>>,
    last_update: HashMap<String, DateTime<Utc>>,
}

#[allow(dead_code)]
impl MarketDataManager {
    /// Create a new MarketDataManager for a list of symbols.
    pub fn new(symbols: &[String]) -> Self {
        MarketDataManager {
            cache: symbols.iter().map(|s| (s.clone(), Vec::new())).collect(),
            last_update: HashMap::new(),
        }
    }

    /// Update market data for all symbols in the cache.
    ///
    /// For each symbol, this method waits 12 seconds (to avoid rate limits), fetches the latest data,
    /// truncates it to the most recent 100 entries, and updates the cache along with the last update timestamp.
    #[allow(dead_code)]
    pub async fn update_data(&mut self) -> Result<(), MarketDataError> {
        let symbols: Vec<String> = self.cache.keys().cloned().collect();

        for symbol in symbols {
            // Wait 12 seconds between API calls to avoid rate limiting.
            tokio::time::sleep(Duration::from_secs(12)).await;

            match fetch_market_data(&symbol).await {
                Ok(mut data) => {
                    // Keep only the 100 most recent data points.
                    data.truncate(100);
                    if let Some(latest) = data.first() {
                        self.last_update.insert(symbol.clone(), latest.0);
                    }
                    self.cache.insert(symbol, data);
                }
                Err(e) => {
                    log::error!("Failed to update {}: {:?}", symbol, e);
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Retrieve cached market data for a given symbol.
    #[allow(dead_code)]
    pub fn get_data(&self, symbol: &str) -> Option<&[(DateTime<Utc>, MinuteData)]> {
        self.cache.get(symbol).map(|v| v.as_slice())
    }

    /// Get the timestamp of the last update for a specific symbol.
    pub fn last_update(&self, symbol: &str) -> Option<DateTime<Utc>> {
        self.last_update.get(symbol).copied()
    }
}
