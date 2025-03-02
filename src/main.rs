mod market_data;
mod order_book;
mod matching_engine;
mod risk_management;

use crate::market_data::fetch_market_data;

use std::sync::Arc;
use std::time::Duration;
use crate::{
    order_book::{Order, OrderBook, OrderType, OrderSide},
    matching_engine::{MatchingEngine, EngineMessage},
    risk_management::RiskManager,
};
use tokio::signal;
use rand::Rng;

#[tokio::main]
async fn main() {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();
    
    // Initialize core components
    let order_book = Arc::new(OrderBook::new());
    let (matching_engine, engine_tx) = MatchingEngine::new(order_book.clone());
    let risk_manager = Arc::new({
        let rm = RiskManager::new(1_000_000.0);
        rm.set_position_limit("AAPL", 10_000.0);
        rm
    });

    // Start market data stream
    tokio::spawn({
        let engine_tx_clone = engine_tx.clone();
        let _order_book = order_book.clone(); // Prefix with underscore to avoid warning
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                if let Ok(data) = fetch_market_data("AAPL").await {
                    log::info!("Received {} market data points", data.len());
                    for (_, md) in data {
                        let orders = md.to_orders("AAPL");
                        for order in orders {
                            engine_tx_clone.send(EngineMessage::NewOrder(order)).unwrap();
                        }
                    }
                }
            }
        }
    });

    // Start position monitoring
    tokio::spawn({
        let risk_manager = risk_manager.clone();
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let position = risk_manager.get_position("AAPL");
                log::info!("Current AAPL position: {:.2}", position);
            }
        }
    });

    // Start spread monitor
    tokio::spawn({
        let order_book = order_book.clone();
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
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

    // Start shutdown listener
    let shutdown = async {
        signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
        log::info!("Shutting down HFT simulator");
    };

    // Order generation loop
    let order_loop = async {
        let order_id = 1; // Removed 'mut' as it's not modified in this example
        let mut rng = rand::thread_rng();
        
        loop {
            let order = Order {
                id: order_id,
                symbol: "AAPL".to_string(),
                price: 150.0 + rng.gen::<f64>() * 5.0,
                quantity: 100.0,
                order_type: OrderType::Limit,
                side: if rng.gen() { OrderSide::Buy } else { OrderSide::Sell },
                timestamp: chrono::Utc::now().timestamp(),
            };

            if risk_manager.validate_order(&order) {
                engine_tx.send(EngineMessage::NewOrder(order.clone())).unwrap();
                
                // 25% chance to cancel after 1 second
                if rand::random::<f64>() < 0.25 {
                    let tx = engine_tx.clone();
                    let symbol = order.symbol.clone();
                    let order_id = order.id;
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        tx.send(EngineMessage::CancelOrder { symbol, order_id }).unwrap();
                    });
                }
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };

    tokio::select! {
        _ = order_loop => {},
        _ = shutdown => {},
    }
}