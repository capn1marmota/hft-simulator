mod market_data;
mod order_book;
mod matching_engine;
mod risk_management;

use std::sync::Arc;
use crate::market_data::fetch_market_data;
use crate::{
    order_book::{Order, OrderBook, OrderType, OrderSide},
    matching_engine::MatchingEngine,
    risk_management::RiskManager,
};
use tokio::signal;

#[tokio::main]
async fn main() {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();
    
    // Initialize core components
    let order_book = Arc::new(OrderBook::new());
    let (matching_engine, order_tx) = MatchingEngine::new(order_book.clone());
    let risk_manager = Arc::new({
        let rm = RiskManager::new(1_000_000.0);
        rm.set_position_limit("AAPL", 10_000.0);  // Set position limit for AAPL
        rm
    });

    // Start market data stream
    tokio::spawn({
        let order_book = order_book.clone();
        async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                if let Ok(data) = fetch_market_data("AAPL").await {
                    log::info!("Received {} market data points", data.len());
                    // Update order book with market data (implementation needed)
                }
            }
        }
    });

    // Start spread monitor
    tokio::spawn({
        let order_book = order_book.clone();
        async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                if let (Some(bid), Some(ask)) = (
                    order_book.get_best_bid("AAPL"),
                    order_book.get_best_ask("AAPL")
                ) {
                    log::info!("Spread: {:.2} - {:.2} ({:.2})", bid, ask, ask - bid);
                }
            }
        }
    });

    // Start matching engine
    tokio::spawn(async move {
        matching_engine.run().await;
    });

    // Order generation loop
    let mut order_id = 1;
    loop {
        let order = Order {
            id: order_id,
            symbol: "AAPL".to_string(),
            price: 150.0 + rand::random::<f64>() * 5.0,
            quantity: 100.0,
            order_type: OrderType::Limit,
            side: if rand::random() { OrderSide::Buy } else { OrderSide::Sell },
            timestamp: chrono::Utc::now().timestamp(),
        };

        if risk_manager.validate_order(&order) {
            risk_manager.update_position(&order);
            order_tx.send(order).unwrap();
            order_id += 1;
        }

        // Add cancellation chance
        if rand::random::<f64>() < 0.1 {
            // Implement order cancellation logic
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Graceful shutdown handling
    signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
    log::info!("Shutting down HFT simulator");
}