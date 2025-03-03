mod market_data;
mod matching_engine;
mod order_book;
mod risk_management;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use tokio::sync::Mutex;

use crate::market_data::fetch_market_data;
use crate::{
    matching_engine::{EngineMessage, MatchingEngine},
    order_book::{Order, OrderBook, OrderSide, OrderType},
    risk_management::RiskManager,
};
use rand::Rng;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    // Initialize shared HTTP client
    let http_client = Arc::new(Client::new());

    // Initialize order book
    let order_book = Arc::new(OrderBook::new());

    // Create the risk manager first
    let risk_manager = Arc::new({
        let rm = RiskManager::new(Decimal::from(1_000_000));
        rm.set_position_limit("AAPL", Decimal::from(10_000));
        rm
    });

    // Create the matching engine
    let (matching_engine, engine_tx, message_rx) =
        MatchingEngine::new(order_book.clone(), risk_manager.clone());
    let matching_engine = Arc::new(matching_engine);

    // Market data stream (now using `reqwest::Client`)
    tokio::spawn({
        let engine_tx_clone = engine_tx.clone();
        let http_client = http_client.clone();
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                match fetch_market_data(&http_client, "AAPL").await {
                    Ok(data) => {
                        log::info!("Received {} market data points", data.len());
                        for (_, md) in data.iter() {
                            let orders = md.to_orders("AAPL", Decimal::new(1, 2));
                            for order in orders {
                                if let Err(e) = engine_tx_clone.send(EngineMessage::NewOrder(order))
                                {
                                    log::error!("Failed to send order: {:?}", e);
                                }
                            }
                        }
                    }
                    Err(e) => log::error!("Market data fetch failed: {:?}", e),
                }
            }
        }
    });

    // Matching engine position reporting
    let message_rx = Arc::new(Mutex::new(message_rx));
    tokio::spawn({
        let engine = Arc::clone(&matching_engine);
        let message_rx = Arc::clone(&message_rx);
        async move {
            let mut lock = message_rx.lock().await;
            engine.run(&mut *lock).await;
        }
    });

    // Spread monitoring (best bid/ask)
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

    // Market data updates in order book (now concurrent)
    tokio::spawn({
        let order_book = order_book.clone();
        let http_client = http_client.clone();
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                match fetch_market_data(&http_client, "AAPL").await {
                    Ok(data) => {
                        log::info!("Processing {} market data entries", data.len());
                        for (_ts, md) in data {
                            order_book.update_from_market_data("AAPL", &md, Decimal::new(1, 2));
                        }
                    }
                    Err(e) => log::error!("Market data update failed: {:?}", e),
                }
            }
        }
    });

    // Risk manager reporting
    tokio::spawn({
        let risk_manager = risk_manager.clone();
        let order_book = order_book.clone();
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                risk_manager.report_positions(|symbol| order_book.get_mid_price(symbol));
            }
        }
    });

    // Shutdown listener
    let shutdown = async {
        signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
        log::info!("Shutting down HFT simulator");
    };

    // Order generation loop with randomized orders
    let order_loop = async {
        let mut rng = rand::thread_rng();
        loop {
            let price = Decimal::from_f64(150.0 + rng.gen::<f64>() * 5.0).unwrap_or(Decimal::ZERO);
            let quantity = Decimal::from(100);

            let order = Order {
                id: Uuid::new_v4().as_u128() as u64,
                symbol: "AAPL".into(),
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
                if let Err(e) = engine_tx.send(EngineMessage::NewOrder(order.clone())) {
                    log::error!("Failed to send order: {:?}", e);
                }

                // 25% chance to cancel after 1 second
                if rand::random::<f64>() < 0.25 {
                    let tx = engine_tx.clone();
                    let symbol = order.symbol.clone();
                    let order_id = order.id;
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        if let Err(e) = tx.send(EngineMessage::CancelOrder { symbol, order_id }) {
                            log::error!("Failed to cancel order: {:?}", e);
                        }
                    });
                }
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };

    // Run order loop alongside shutdown listener
    tokio::select! {
        _ = order_loop => {},
        _ = shutdown => {},
    }
}
