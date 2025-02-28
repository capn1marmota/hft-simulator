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
    let risk_manager = RiskManager::new(1000.0);
    risk_manager.set_position_limit("AAPL", 5000.0);

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

    // Generate orders continuously
    let mut order_id = 1;
    loop {
        let order = Order {
            id: order_id,
            symbol: "AAPL".to_string(),
            price: 150.0 + rand::random::<f64>() * 5.0,  // Random price between 150-155
            quantity: 100.0,
            order_type: OrderType::Limit,
            side: if rand::random() { OrderSide::Buy } else { OrderSide::Sell },
            timestamp: chrono::Utc::now().timestamp(),
        };

        if risk_manager.validate_order(&order) {
            order_tx.send(order).unwrap();
            order_id += 1;  // Correctly increment order ID
        }

        // Add some delay between orders
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}