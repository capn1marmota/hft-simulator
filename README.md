# HFT Simulator

A high-frequency trading simulator written in Rust that models a complete trading system with matching engine, order book, risk management, and market data integration.

## Features

- **Matching Engine**: Process and execute buy/sell orders
- **Order Book**: Maintain price-time priority for limit orders
- **Risk Management**: Position tracking, P&L calculation, and risk limits
- **Market Data Integration**: Connect to Alpha Vantage API for real-time data
- **Asynchronous Architecture**: Built with Tokio for high-performance concurrency

## Getting Started

### Prerequisites

- Rust 1.63+
- Alpha Vantage API key

### Installation

1. Clone the repository:
   ```
   git clone https://github.com/capn1marmota/hft-simulator.git
   cd hft-simulator
   ```

2. Set your Alpha Vantage API key:
   ```
   export ALPHA_VANTAGE_API_KEY="your_api_key_here"

      ## Setup

   1. Copy `.env.example` to `.env`
   2. Replace the placeholder API key with your actual Alpha Vantage API key
   3. Do not commit your `.env` file
   ```

3. Build the project:
   ```
   cargo build
   ```

### Running the Simulator



```
cargo run
```

The simulator will:
- Generate random market and limit orders
- Process these orders through the matching engine
- Display spread information and order execution details
- Report risk positions and P&L

Press Ctrl+C to gracefully shut down the simulator.

## Architecture

- **Order Book**: Thread-safe implementation using DashMap for concurrent access
- **Matching Engine**: Processes orders and maintains a queue of execution messages
- **Risk Manager**: Enforces position limits and tracks P&L
- **Market Data**: Fetches real-time data and converts to orders

## Configuration

- Default risk limit: $1,000,000
- Default position limit for AAPL: 10,000 shares
- Market data refresh: Every 60 seconds
- Spread monitoring: Every 5 seconds
- Risk reporting: Every 10 seconds

## License

[MIT License](LICENSE)

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
