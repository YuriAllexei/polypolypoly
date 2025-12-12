//! Test binary for order placement
//!
//! Tests the EIP-712 signed order placement functionality using TradingClient.
//!
//! Requires environment variables in `.env`:
//!   - PRIVATE_KEY (with 0x prefix) - the signer address is derived from this
//!   - PROXY_WALLET (optional - your Polymarket proxy wallet from reveal.polymarket.com)
//!   - API_KEY, API_SECRET, API_PASSPHRASE (optional - will be derived if not provided)
//!
//! Usage:
//!   cargo run --bin test_order -- <token_id> <price> <size> <buy|sell> [order_type]
//!
//! Examples:
//!   # Place a GTC buy order for 10 tokens at $0.50
//!   cargo run --bin test_order -- 123456... 0.50 10 buy
//!
//!   # Place a FOK sell order
//!   cargo run --bin test_order -- 123456... 0.60 5 sell fok

use anyhow::{bail, Result};
use polymarket::infrastructure::client::clob::{OrderType, Side, TradingClient};
use std::env;
use tracing::error;

fn print_usage(program: &str) {
    println!("Order Placement Test");
    println!();
    println!(
        "Usage: {} <token_id> <price> <size> <side> [order_type]",
        program
    );
    println!();
    println!("Arguments:");
    println!("  token_id    ERC1155 conditional token ID");
    println!("  price       Price per token (0.01 to 0.99)");
    println!("  size        Number of tokens to buy/sell");
    println!("  side        'buy' or 'sell'");
    println!("  order_type  Optional: 'gtc' (default), 'fok', 'gtd', or 'fak'");
    println!();
    println!("Environment Variables (set in .env file):");
    println!("  PRIVATE_KEY     Your Ethereum private key (0x prefixed)");
    println!(
        "  PROXY_WALLET    Your Polymarket proxy wallet (optional, from reveal.polymarket.com)"
    );
    println!("  API_KEY         Polymarket API key (optional - will be derived if not provided)");
    println!("  API_SECRET      Polymarket API secret (optional)");
    println!("  API_PASSPHRASE  Polymarket API passphrase (optional)");
    println!();
    println!("Examples:");
    println!("  # Buy 10 tokens at $0.50 with GTC order");
    println!("  {} 8861317280354... 0.50 10 buy", program);
    println!();
    println!("  # Sell 5 tokens at $0.60 with FOK order");
    println!("  {} 8861317280354... 0.60 5 sell fok", program);
}

fn parse_side(s: &str) -> Result<Side> {
    match s.to_lowercase().as_str() {
        "buy" | "b" => Ok(Side::Buy),
        "sell" | "s" => Ok(Side::Sell),
        _ => bail!("Invalid side '{}'. Use 'buy' or 'sell'", s),
    }
}

fn parse_order_type(s: &str) -> Result<OrderType> {
    match s.to_lowercase().as_str() {
        "gtc" => Ok(OrderType::GTC),
        "fok" => Ok(OrderType::FOK),
        "gtd" => Ok(OrderType::GTD),
        "fak" => Ok(OrderType::FAK),
        _ => bail!(
            "Invalid order type '{}'. Use 'gtc', 'fok', 'gtd', or 'fak'",
            s
        ),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let args: Vec<String> = env::args().collect();

    if args.len() < 5 {
        print_usage(&args[0]);
        return Ok(());
    }

    // Parse arguments
    let token_id = &args[1];
    let price: f64 = args[2].parse()?;
    let size: f64 = args[3].parse()?;
    let side = parse_side(&args[4])?;
    let order_type = if args.len() > 5 {
        parse_order_type(&args[5])?
    } else {
        OrderType::GTC
    };

    println!();
    println!("════════════════════════════════════════════════════════════════");
    println!("ORDER PLACEMENT TEST");
    println!("════════════════════════════════════════════════════════════════");
    println!();
    println!("Order Parameters:");
    println!("  Token ID:   {}...", &token_id[..20.min(token_id.len())]);
    println!("  Price:      ${:.4}", price);
    println!("  Size:       {:.2} tokens", size);
    println!("  Side:       {:?}", side);
    println!("  Order Type: {:?}", order_type);
    println!("  Est. Cost:  ${:.4}", price * size);
    println!();

    // Initialize trading client (handles all auth setup automatically)
    println!("Initializing trading client...");
    let client = TradingClient::from_env().await?;

    println!("  Signer:     {:?}", client.signer_address());
    println!("  Maker:      {:?}", client.maker_address());
    println!("  Sig Type:   Gnosis Safe (2)");
    println!();

    println!("════════════════════════════════════════════════════════════════");
    println!("PLACING ORDER...");
    println!("════════════════════════════════════════════════════════════════");
    println!();

    // Place the order
    let result = client
        .place_order(token_id, price, size, side, order_type)
        .await;

    match result {
        Ok(response) => {
            println!("ORDER RESULT:");
            println!("────────────────────────────────────────────────────────────────");
            println!("  Success:    {}", response.success);
            if let Some(order_id) = &response.order_id {
                println!("  Order ID:   {}", order_id);
            }
            if let Some(status) = &response.status {
                println!("  Status:     {}", status);
            }
            if let Some(hashes) = &response.order_hashes {
                if !hashes.is_empty() {
                    println!("  Tx Hashes:  {:?}", hashes);
                }
            }
            if let Some(err) = &response.error_msg {
                if !err.is_empty() {
                    println!("  Error:      {}", err);
                }
            }
            println!();

            if response.success {
                println!("Order placed successfully!");
            } else {
                error!("Order placement failed");
            }
        }
        Err(e) => {
            println!("ORDER FAILED:");
            println!("────────────────────────────────────────────────────────────────");
            println!("  Error: {}", e);
            println!();
            error!("Order placement error: {}", e);
        }
    }

    println!();
    println!("════════════════════════════════════════════════════════════════");

    Ok(())
}
