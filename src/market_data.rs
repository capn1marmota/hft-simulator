use serde::Deserialize;
use reqwest::Error;
use chrono::{DateTime, Utc};

#[derive(Debug, Deserialize)]
struct AlphaVantageResponse {
    #[serde(rename = "Time Series (1min)")]
    time_series: std::collections::HashMap<String, MinuteData>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]  // Temporary until market data integration
pub struct MinuteData {
    #[serde(rename = "1. open")]
    open: f64,
    #[serde(rename = "2. high")]
    high: f64,
    #[serde(rename = "4. close")]
    close: f64,
    #[serde(rename = "5. volume")]
    volume: f64,
}

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
