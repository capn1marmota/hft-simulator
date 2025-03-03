mod market_data;
mod matching_engine;
mod order_book;
mod risk_management;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;

use crate::market_data::fetch_market_data;
use crate::{
    matching_engine::{EngineMessage, MatchingEngine},
    order_book::{Order, OrderBook, OrderSide, OrderType},
    risk_management::RiskManager,
};
use rand::Rng;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    // Initialize core components
    let order_book = Arc::new(OrderBook::new());

    // Create the risk manager first
    let risk_manager = Arc::new({
        let rm = RiskManager::new(Decimal::from(1_000_000));
        rm.set_position_limit("AAPL", Decimal::from(10_000));
        rm
    });

    // Then create the matching engine
    let (matching_engine, engine_tx, message_rx) =
        MatchingEngine::new(order_book.clone(), risk_manager.clone());
    let matching_engine = Arc::new(matching_engine);

    // Start market data stream
    tokio::spawn({
        let engine_tx_clone = engine_tx.clone();
        let _order_book = order_book.clone(); // Suppress unused variable warning
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                if let Ok(data) = fetch_market_data("AAPL").await {
                    log::info!("Received {} market data points", data.len());
                    for (_, md) in data.iter() {
                        let orders = md.to_orders("AAPL", Decimal::new(1, 2));
                        for order in orders {
                            engine_tx_clone
                                .send(EngineMessage::NewOrder(order))
                                .unwrap();
                        }
                    }
                }
            }
        }
    });

    // Start comprehensive reporting (matching engine positions)
    tokio::spawn({
        let engine = matching_engine.clone();
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                engine.report_positions();
            }
        }
    });

    // Start spread monitor (order book best bid/ask)
    tokio::spawn({
        let order_book = order_book.clone();
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                if let (Some(bid), Some(ask)) = (
                    order_book.get_best_bid("AAPL"),
                    order_book.get_best_ask("AAPL"),
                ) {
                    log::info!("Spread: {:.2} - {:.2} ({:.2})", bid, ask, ask - bid);
                }
            }
        }
    });

    // Update market data in order book
    tokio::spawn({
        let order_book = order_book.clone();
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                if let Ok(data) = fetch_market_data("AAPL").await {
                    log::info!("Processing {} market data entries", data.len());
                    for (_ts, md) in data {
                        order_book.update_from_market_data("AAPL", &md, Decimal::new(1, 2));
                    }
                }
            }
        }
    });

    // Start the matching engine processing loop
    {
        let engine_clone = matching_engine.clone();
        tokio::spawn(async move {
            // Cloning the engine here to satisfy ownership; adjust as needed.
            let engine = engine_clone.as_ref().clone();
            engine.run(message_rx).await;
        });
    }

    // Start risk manager reporting positions (using mid price from order book)
    {
        let risk_manager_clone = risk_manager.clone();
        let order_book_clone = order_book.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                risk_manager_clone
                    .report_positions(|symbol| order_book_clone.get_mid_price(symbol));
            }
        });
    }

    // Shutdown listener
    let shutdown = async {
        signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
        log::info!("Shutting down HFT simulator");
    };

    // Order generation loop
    let order_loop = async {
        let mut rng = rand::thread_rng();

        loop {
            let price = Decimal::from_f64(150.0 + rng.gen::<f64>() * 5.0).unwrap_or(Decimal::ZERO);
            let quantity = Decimal::from(100);

            let order = Order {
                id: Uuid::new_v4().as_u128() as u64,
                symbol: "AAPL".to_string(),
                price,
                quantity,
                order_type: OrderType::Limit,
                side: if rng.gen() {
                    OrderSide::Buy
                } else {
                    OrderSide::Sell
                },
                timestamp: chrono::Utc::now().timestamp(),
            };

            if risk_manager.validate_order(&order) {
                engine_tx
                    .send(EngineMessage::NewOrder(order.clone()))
                    .unwrap();

                // 25% chance to cancel after 1 second
                if rand::random::<f64>() < 0.25 {
                    let tx = engine_tx.clone();
                    let symbol = order.symbol.clone();
                    let order_id = order.id;
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        tx.send(EngineMessage::CancelOrder { symbol, order_id })
                            .unwrap();
                    });
                }
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };

    // Run order generation loop alongside shutdown signal listener
    tokio::select! {
        _ = order_loop => {},
        _ = shutdown => {},
    }
}
