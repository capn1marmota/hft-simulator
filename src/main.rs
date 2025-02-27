mod market_data;
mod order_book;
mod matching_engine;
mod risk_management;

use crate::market_data::fetch_market_data;
use std::sync::Arc;
use crate::{
    order_book::{Order, OrderBook, OrderType, OrderSide},
    matching_engine::MatchingEngine,
    risk_management::RiskManager,
};

#[tokio::main]
async fn main() {
    env_logger::Builder::new()
    .filter_level(log::LevelFilter::Info)
    .init();
    
    // Initialize core components
    let order_book = Arc::new(OrderBook::new());
    let (matching_engine, order_tx) = MatchingEngine::new(order_book.clone());
    let risk_manager = Arc::new(RiskManager::new(1_000_000.0));

    // Start market data stream
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Ok(data) = fetch_market_data("AAPL").await {
                // Update order book with market data
                log::info!("Received {} market data points", data.len());
            }
        }
    });

    // Start matching engine
    tokio::spawn(async move {
        matching_engine.run().await;
    });

    // Example order submission
    let order = Order {
        id: 1,
        symbol: "AAPL".to_string(),
        price: 150.0,
        quantity: 100.0,
        order_type: OrderType::Limit,
        side: OrderSide::Buy,
        timestamp: chrono::Utc::now().timestamp(),
    };

    if risk_manager.validate_order(&order) {
        order_tx.send(order).unwrap();
    }

    // Keep the main thread alive
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}