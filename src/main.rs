mod market_data;
mod matching_engine;
mod order_book;
mod risk_management;

use crate::{
    market_data::{fetch_market_data, EfficientMarketDataBuffer, MarketDataManager},
    matching_engine::{EngineMessage, MatchingEngine},
    order_book::{Order, OrderBook, OrderSide, OrderType},
    risk_management::RiskManager,
};
use rand::Rng;
use reqwest::Client;
use rust_decimal::{prelude::FromPrimitive, Decimal};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{signal, sync::Mutex};

// Define a static atomic counter for unique order IDs
static ORDER_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[tokio::main]
async fn main() {
    // Initialize logging
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    // Load .env file
    dotenv::dotenv().ok();

    // Shared HTTP client for market data
    let http_client = Arc::new(Client::new());

    // Shared order book
    let order_book = Arc::new(OrderBook::new());

    // Initialize risk manager with position limits
    let risk_manager = Arc::new({
        let rm = RiskManager::new(Decimal::from(1_000_000));
        rm.set_position_limit("AAPL", Decimal::from(10_000));
        rm
    });

    // Initialize Efficient Market Data Buffer
    let market_data_buffer = Arc::new(EfficientMarketDataBuffer::new(100));

    // Initialize matching engine
    let (matching_engine, engine_tx, message_rx) =
        MatchingEngine::new(order_book.clone(), risk_manager.clone());
    let matching_engine = Arc::new(matching_engine);

    // Market data task: Convert market data to orders and send to matching engine
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

    // Matching engine task: Process messages
    let message_rx = Arc::new(Mutex::new(message_rx));
    tokio::spawn({
        let engine = matching_engine.clone();
        let message_rx = message_rx.clone();
        async move {
            let mut lock = message_rx.lock().await;
            engine.run(&mut *lock).await;
        }
    });

    // Spread monitoring task: Log best bid/ask every 5 seconds
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

    // Risk manager reporting task: Log positions every 10 seconds
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

    // Market Data Manager task: Periodically update market data
    let mut market_data_manager = MarketDataManager::new(&["AAPL".to_string()]);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(e) = market_data_manager.update_data().await {
                log::error!("Market data update error: {:?}", e);
            }
        }
    });

    // Market Data Buffer Analysis Task: Periodically analyze buffered data
    tokio::spawn({
        let market_data_buffer = market_data_buffer.clone();
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let recent_data = market_data_buffer.get_recent_data();

                if !recent_data.is_empty() {
                    // Simple analysis: calculate average close price
                    let avg_close: Decimal = recent_data
                        .iter()
                        .map(|(_, data)| data.close)
                        .sum::<Decimal>()
                        / Decimal::from(recent_data.len());

                    log::info!("Recent data average close price: {:.2}", avg_close);
                }
            }
        }
    });

    matching_engine.start_reporting(10).await;

    // Shutdown listener
    let shutdown = async {
        signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
        log::info!("Shutting down HFT simulator");
    };

    // Order generation loop: Create and send random orders
    let order_loop = async {
        let mut rng = rand::thread_rng();
        loop {
            let price = Decimal::from_f64(rng.gen_range(100.0..200.0)).unwrap_or(Decimal::ZERO);
            let quantity = Decimal::from(rng.gen_range(10..1001));

            let order = Order {
                id: ORDER_ID_COUNTER.fetch_add(1, Ordering::Relaxed),
                symbol: "AAPL".into(),
                price,
                quantity,
                order_type: OrderType::Limit,
                side: if rng.gen() {
                    OrderSide::Buy
                } else {
                    OrderSide::Sell
                },
                timestamp: chrono::Utc::now()
                    .timestamp_nanos_opt()
                    .expect("Failed to get nanosecond timestamp"),
            };

            if risk_manager.validate_order(&order) {
                if let Err(e) = engine_tx.send(EngineMessage::NewOrder(order.clone())) {
                    log::error!("Failed to send order: {:?}", e);
                }

                // 25% chance to cancel the order after 1 second
                if rng.gen::<f64>() < 0.25 {
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

    // Run order loop and shutdown listener concurrently
    tokio::select! {
        _ = order_loop => {},
        _ = shutdown => {},
    }
}
