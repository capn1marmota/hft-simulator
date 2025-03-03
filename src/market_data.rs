use serde::Deserialize;
use reqwest::Error;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use crate::{
    order_book::{Order, OrderType, OrderSide},
};

#[derive(Debug, Deserialize)]
struct AlphaVantageResponse {
    #[serde(rename = "Time Series (1min)")]
    time_series: HashMap<String, MinuteData>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Temporary for API compatibility
pub struct MinuteData {
    #[serde(rename = "1. open")]
    pub open: f64,
    #[serde(rename = "2. high")]
    pub high: f64,
    #[serde(rename = "4. close")]
    pub close: f64,
    #[serde(rename = "5. volume")]
    pub volume: f64,
}

/// Fetches market data from AlphaVantage for the given symbol.
/// Returns a vector of (timestamp, MinuteData) pairs.
pub async fn fetch_market_data(symbol: &str) -> Result<Vec<(DateTime<Utc>, MinuteData)>, Error> {
    let api_key = "4RMSF2E3473M9N5J";
    let url = format!(
        "https://www.alphavantage.co/query?function=TIME_SERIES_INTRADAY&symbol={}&interval=1min&apikey={}",
        symbol, api_key
    );

    let response = reqwest::get(&url).await?.json::<AlphaVantageResponse>().await?;

    let mut data = Vec::new();
    for (timestamp, values) in response.time_series {
        let dt = DateTime::parse_from_str(&timestamp, "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .with_timezone(&Utc);
        data.push((dt, values));
    }

    Ok(data)
}

impl MinuteData {
    /// Converts a minute's market data into a pair of limit orders:
    /// one buy order (with price slightly lower than the close)
    /// and one sell order (with price slightly higher than the close).
    pub fn to_orders(&self, symbol: &str) -> Vec<Order> {
        let spread = 0.1;
        let ts = Utc::now().timestamp();
        vec![
            Order {
                id: (ts * 1000) as u64,
                symbol: symbol.to_string(),
                price: self.close - spread,
                quantity: 100.0,
                order_type: OrderType::Limit,
                side: OrderSide::Buy,
                timestamp: ts,
            },
            Order {
                id: (ts * 1000 + 1) as u64,
                symbol: symbol.to_string(),
                price: self.close + spread,
                quantity: 100.0,
                order_type: OrderType::Limit,
                side: OrderSide::Sell,
                timestamp: ts,
            }
        ]
    }
}
