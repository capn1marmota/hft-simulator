use tokio::sync::mpsc;
use crate::order_book::{Order, OrderBook, OrderType, OrderSide};
use std::sync::Arc;
use log::warn;
use ordered_float::OrderedFloat;
use crate::RiskManager;

pub enum EngineMessage {
    NewOrder(Order),
    CancelOrder { symbol: String, order_id: u64 },
}

pub struct MatchingEngine {
    order_book: Arc<OrderBook>,
    risk_manager: Arc<RiskManager>,
    message_rx: mpsc::UnboundedReceiver<EngineMessage>,
}

impl MatchingEngine {
    pub fn new(order_book: Arc<OrderBook>,
    risk_manager: Arc<RiskManager>) -> (Self, mpsc::UnboundedSender<EngineMessage>) {
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        (Self { order_book, message_rx, risk_manager, }, message_tx)
    }

    pub async fn run(mut self) {
        while let Some(msg) = self.message_rx.recv().await {
            match msg {
                EngineMessage::NewOrder(order) => {
                    self.process_order(order).await;
                }
                EngineMessage::CancelOrder { symbol, order_id } => {
                    self.process_cancellation(&symbol, order_id).await;
                }
            }
        }
    }

    async fn process_cancellation(&self, symbol: &str, order_id: u64) {
        let cancelled = self.order_book.cancel_order(symbol, order_id);
        if cancelled {
            log::info!("Cancelled order {}", order_id);
        } else {
            log::warn!("Failed to cancel order {}", order_id);
        }
    }

    async fn process_order(&self, mut order: Order) {
        let symbol = order.symbol.clone();
        let remaining_qty = order.quantity;

        // Log order receipt
        log::debug!("Processing order {}: {:?}", order.id, order);
    
        match order.order_type {
            OrderType::Limit => {
                if order.price > 0.0 {
                    let is_bid = match order.side {
                        OrderSide::Buy => true,
                        OrderSide::Sell => false,
                    };
                    self.process_limit_order(&symbol, &order, remaining_qty, is_bid).await
                } else {
                    warn!("Limit order with invalid price: {}", order.price);
                }
            },
            OrderType::Market => {
                let is_bid = match order.side {
                    OrderSide::Buy => true,
                    OrderSide::Sell => false,
                };
                self.process_market_order(&symbol, &order, remaining_qty, is_bid).await
            }
        }

        if remaining_qty > 0.0 {
            let fill_qty = order.quantity - remaining_qty; // Calculate how much was filled
            self.risk_manager.update_position(&order, fill_qty);
            log::debug!("Adding remaining {} to book", remaining_qty);
            order.quantity = remaining_qty; // Modify original order
            self.order_book.add_order(order);  // Pass ownership
        }
    }

    async fn process_limit_order(
        &self,
        symbol: &str,
        order: &Order,
        mut remaining_qty: f64,
        is_bid: bool,
    ) {
        if is_bid {
            // Process bid order against asks
            if let Some(mut asks) = self.order_book.asks.get_mut(symbol) {
                // Keep track of orders to be removed after matching
                let mut to_remove = Vec::new();
                
                // Iterate through ask price levels in ascending order
                for (price_key, orders) in asks.iter_mut() {
                    let ask_price = *price_key;
                    
                    // Stop matching if ask price exceeds bid price
                    if ask_price > OrderedFloat(order.price) {
                        break;
                    }
                    
                    // Keep track of indices to remove
                    let mut filled_indices = Vec::new();
                    
                    // Match orders at this price level
                    for (idx, ask_order) in orders.iter_mut().enumerate() {
                        let match_qty = remaining_qty.min(ask_order.quantity);
                        
                        // Execute the trade
                        log::info!("Match: Bid {} x {} @ {}", match_qty, symbol, ask_price);

                        
                        // Update quantities
                        remaining_qty -= match_qty;
                        ask_order.quantity -= match_qty;
                        
                        // If ask order is fully filled, mark for removal
                        if ask_order.quantity <= 0.0 {
                            filled_indices.push(idx);
                        }
                        
                        // If incoming order is fully matched, exit
                        if remaining_qty <= 0.0 {
                            break;
                        }
                    }
                    
                    // Remove filled orders (in reverse to maintain indices)
                    for idx in filled_indices.iter().rev() {
                        orders.remove(*idx);
                    }
                    
                    // If price level is empty, mark for removal
                    if orders.is_empty() {
                        to_remove.push(*price_key);
                    }
                    
                    // Exit if incoming order fully matched
                    if remaining_qty <= 0.0 {
                        break;
                    }
                }
                
                // Remove empty price levels
                for price in to_remove {
                    asks.remove(&price);
                }
            }
        } else {
            // Process ask order against bids
            if let Some(mut bids) = self.order_book.bids.get_mut(symbol) {
                // Keep track of price levels to be removed
                let mut to_remove = Vec::new();
                
                // Iterate through bid price levels in descending order (highest first)
                for (price_key, orders) in bids.iter_mut() {
                    let bid_price = price_key.0.into_inner();
                    
                    // Stop matching if bid price is below ask price
                    if bid_price < order.price {
                        break;
                    }
                    
                    // Keep track of indices to remove
                    let mut filled_indices = Vec::new();
                    
                    // Match orders at this price level
                    for (idx, bid_order) in orders.iter_mut().enumerate() {
                        let match_qty = remaining_qty.min(bid_order.quantity);
                        
                        // Execute the trade
                        log::info!("Match: Ask {} x {} @ {}", match_qty, symbol, bid_price);
                        
                        // Update quantities
                        remaining_qty -= match_qty;
                        bid_order.quantity -= match_qty;
                        
                        // If bid order is fully filled, mark for removal
                        if bid_order.quantity <= 0.0 {
                            filled_indices.push(idx);
                        }
                        
                        // If incoming order is fully matched, exit
                        if remaining_qty <= 0.0 {
                            break;
                        }
                    }
                    
                    // Remove filled orders (in reverse to maintain indices)
                    for idx in filled_indices.iter().rev() {
                        orders.remove(*idx);
                    }
                    
                    // If price level is empty, mark for removal
                    if orders.is_empty() {
                        to_remove.push(*price_key);
                    }
                    
                    // Exit if incoming order fully matched
                    if remaining_qty <= 0.0 {
                        break;
                    }
                }
                
                // Remove empty price levels
                for price in to_remove {
                    bids.remove(&price);
                }
            }
        }
    
        // Add remaining order to the book if not fully matched
        if remaining_qty > 0.0 {
            let fill_qty = order.quantity - remaining_qty; // Calculate how much was filled
            self.risk_manager.update_position(&order, fill_qty);
            let mut new_order = order.clone();
            new_order.quantity = remaining_qty;
            self.order_book.add_order(new_order);
        }
    }

    async fn process_market_order(
        &self,
        symbol: &str,
        order: &Order,
        mut remaining_qty: f64,
        is_bid: bool,
    ) {
        if is_bid {
            // Process market buy against asks
            if let Some(mut asks) = self.order_book.asks.get_mut(symbol) {
                let mut to_remove = Vec::new();
                
                // Market orders take liquidity at any price, so we iterate through all price levels
                for (price_key, orders) in asks.iter_mut() {
                    let ask_price = price_key.into_inner();
                    let mut filled_indices = Vec::new();
                    
                    // Match orders at this price level
                    for (idx, ask_order) in orders.iter_mut().enumerate() {
                        let match_qty = remaining_qty.min(ask_order.quantity);
                        
                        // Execute the trade
                        log::info!("Market Buy: {} x {} @ {}", match_qty, symbol, ask_price);
                        
                        // Update quantities
                        remaining_qty -= match_qty;
                        ask_order.quantity -= match_qty;
                        
                        // If ask order is fully filled, mark for removal
                        if ask_order.quantity <= 0.0 {
                            filled_indices.push(idx);
                        }
                        
                        // If incoming order is fully matched, exit
                        if remaining_qty <= 0.0 {
                            break;
                        }
                    }
                    
                    // Remove filled orders (in reverse to maintain indices)
                    for idx in filled_indices.iter().rev() {
                        orders.remove(*idx);
                    }
                    
                    // If price level is empty, mark for removal
                    if orders.is_empty() {
                        to_remove.push(*price_key);
                    }
                    
                    // Exit if incoming order fully matched
                    if remaining_qty <= 0.0 {
                        break;
                    }
                }
                
                // Remove empty price levels
                for price in to_remove {
                    asks.remove(&price);
                }
            }
        } else {
            // Process market sell against bids
            if let Some(mut bids) = self.order_book.bids.get_mut(symbol) {
                let mut to_remove = Vec::new();
                
                // Market orders take liquidity at any price, so we iterate through all price levels
                for (price_key, orders) in bids.iter_mut() {
                    let bid_price = price_key.0.into_inner();
                    let mut filled_indices = Vec::new();
                    
                    // Match orders at this price level
                    for (idx, bid_order) in orders.iter_mut().enumerate() {
                        let match_qty = remaining_qty.min(bid_order.quantity);
                        
                        // Execute the trade
                        println!("Market Sell: {} x {} @ {}", match_qty, symbol, bid_price);
                        
                        // Update quantities
                        remaining_qty -= match_qty;
                        bid_order.quantity -= match_qty;
                        
                        // If bid order is fully filled, mark for removal
                        if bid_order.quantity <= 0.0 {
                            filled_indices.push(idx);
                        }
                        
                        // If incoming order is fully matched, exit
                        if remaining_qty <= 0.0 {
                            break;
                        }
                    }
                    
                    // Remove filled orders (in reverse to maintain indices)
                    for idx in filled_indices.iter().rev() {
                        orders.remove(*idx);
                    }
                    
                    // If price level is empty, mark for removal
                    if orders.is_empty() {
                        to_remove.push(*price_key);
                    }
                    
                    // Exit if incoming order fully matched
                    if remaining_qty <= 0.0 {
                        break;
                    }
                }
                
                // Remove empty price levels
                for price in to_remove {
                    bids.remove(&price);
                }
            }
        }
    
        // For market orders, we don't add the remaining quantity to the book
        // Instead, we can either reject the unfilled portion or report it
        if remaining_qty > 0.0 {
            let fill_qty = order.quantity - remaining_qty; // Calculate how much was filled
            self.risk_manager.update_position(&order, fill_qty);
            println!("Market order partially filled: {} remaining out of {}", 
                     remaining_qty, order.quantity);
        }
    }
}