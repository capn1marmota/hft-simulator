use crate::order_book::{Order, OrderSide, OrderType};
use chrono::{DateTime, Utc};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};
use std::{collections::HashMap, str::FromStr, time::Duration};

/// Custom error type for market data fetching.
#[derive(Debug)]
pub enum MarketDataError {
    MissingApiKey,
    Reqwest(reqwest::Error),
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

/// Fetch market data from Alpha Vantage using a shared `reqwest::Client` for efficiency.
pub async fn fetch_market_data(
    client: &Client,
    symbol: &str,
) -> Result<Vec<(DateTime<Utc>, MinuteData)>, MarketDataError> {
    let api_key =
        std::env::var("ALPHA_VANTAGE_API_KEY").map_err(|_| MarketDataError::MissingApiKey)?;
    let url = format!(
        "https://www.alphavantage.co/query?function=TIME_SERIES_INTRADAY&symbol={}&interval=1min&apikey={}",
        symbol, api_key
    );

    let mut attempts = 3;
    let mut last_error: Option<reqwest::Error> = None;

    while attempts > 0 {
        match client.get(&url).send().await {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    let body = response.text().await.unwrap_or_default();
                    return Err(MarketDataError::ApiError(format!(
                        "Status {}: {}",
                        status, body
                    )));
                }

                let json_response = response.json::<AlphaVantageResponse>().await?;
                let mut data = json_response
                    .time_series
                    .into_iter()
                    .filter_map(|(timestamp, values)| {
                        DateTime::parse_from_str(&timestamp, "%Y-%m-%d %H:%M:%S")
                            .ok()
                            .map(|dt| (dt.with_timezone(&Utc), values))
                    })
                    .collect::<Vec<_>>();

                data.sort_by(|a, b| b.0.cmp(&a.0));
                return Ok(data);
            }
            Err(e) => {
                last_error = Some(e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        attempts -= 1;
    }
    Err(MarketDataError::Reqwest(last_error.unwrap()))
}

impl MinuteData {
    pub fn to_orders(&self, symbol: &str, tick_size: Decimal) -> Vec<Order> {
        let spread_pct = Decimal::new(1, 3);
        let spread = self.close * spread_pct;
        let ts = Utc::now().timestamp_nanos_opt().unwrap_or_default() as u64;
        let base_quantity = (self.volume * Decimal::new(1, 3)).max(Decimal::new(10, 0));

        vec![
            Order {
                id: ts,
                symbol: symbol.into(),
                price: round_to_tick(self.close - spread, tick_size),
                quantity: base_quantity,
                order_type: OrderType::Limit,
                side: OrderSide::Buy,
                timestamp: ts as i64,
            },
            Order {
                id: ts + 1,
                symbol: symbol.into(),
                price: round_to_tick(self.close + spread, tick_size),
                quantity: base_quantity,
                order_type: OrderType::Limit,
                side: OrderSide::Sell,
                timestamp: ts as i64,
            },
        ]
    }
}

/// Rounds the provided price to the nearest multiple of the tick size.
fn round_to_tick(price: Decimal, tick_size: Decimal) -> Decimal {
    (price / tick_size).round() * tick_size
}

/// Market data manager that caches and updates market data.
#[allow(dead_code)]
pub struct MarketDataManager {
    client: Client,
    cache: HashMap<String, Vec<(DateTime<Utc>, MinuteData)>>,
    last_update: HashMap<String, DateTime<Utc>>,
}
#[allow(dead_code)]
impl MarketDataManager {
    pub fn new(symbols: &[String]) -> Self {
        MarketDataManager {
            client: Client::new(),
            cache: symbols.iter().map(|s| (s.clone(), Vec::new())).collect(),
            last_update: HashMap::new(),
        }
    }

    /// Updates market data for all tracked symbols concurrently.
    #[allow(dead_code)]
    pub async fn update_data(&mut self) -> Result<(), MarketDataError> {
        let symbols: Vec<String> = self.cache.keys().cloned().collect();
        let mut tasks = Vec::new();

        for symbol in symbols {
            let client = self.client.clone();
            tasks.push(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(12)).await;
                fetch_market_data(&client, &symbol)
                    .await
                    .map(|data| (symbol, data))
            }));
        }

        for task in tasks {
            match task
                .await
                .unwrap_or_else(|_| Err(MarketDataError::ApiError("Task panicked".into())))
            {
                Ok((symbol, mut data)) => {
                    data.truncate(100);
                    if let Some(latest) = data.first() {
                        self.last_update.insert(symbol.clone(), latest.0);
                    }
                    self.cache.insert(symbol, data);
                }
                Err(e) => log::error!("Failed to update market data: {:?}", e),
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_data(&self, symbol: &str) -> Option<&[(DateTime<Utc>, MinuteData)]> {
        self.cache.get(symbol).map(|v| v.as_slice())
    }

    pub fn last_update(&self, symbol: &str) -> Option<DateTime<Utc>> {
        self.last_update.get(symbol).copied()
    }
}
