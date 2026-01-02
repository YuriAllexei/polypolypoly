//! Test binary for EOA order placement
//!
//! Simplified order placement for EOA (Externally Owned Account) wallets.
//! Unlike POLY_PROXY mode, no proxy wallet is needed - the wallet derived
//! from PRIVATE_KEY is used directly for trading.
//!
//! Requires environment variables in `.env`:
//!   - PRIVATE_KEY (with 0x prefix) - your trading wallet
//!   - API_KEY, API_SECRET, API_PASSPHRASE (optional - will be derived if not provided)
//!
//! Usage:
//!   cargo run --bin test_order_eoa -- <token_id> <price> <size> <buy|sell> [order_type]
//!
//! Examples:
//!   # Place a GTC buy order for 10 tokens at $0.50
//!   cargo run --bin test_order_eoa -- 123456... 0.50 10 buy
//!
//!   # Place a FOK sell order
//!   cargo run --bin test_order_eoa -- 123456... 0.60 5 sell fok

use anyhow::{bail, Context, Result};
use polymarket::infrastructure::client::auth::PolymarketAuth;
use polymarket::infrastructure::client::clob::{
    OrderBuilder, OrderType, RestClient, Side, POLYGON_CHAIN_ID,
};
use std::env;
use tracing::{error, info};

const CLOB_URL: &str = "https://clob.polymarket.com";

fn print_usage(program: &str) {
    println!("EOA Order Placement Test");
    println!();
    println!("Usage: {} <token_id> <price> <size> <side> [order_type]", program);
    println!();
    println!("Arguments:");
    println!("  token_id    ERC1155 conditional token ID");
    println!("  price       Price per token (0.01 to 0.99)");
    println!("  size        Number of tokens to buy/sell");
    println!("  side        'buy' or 'sell'");
    println!("  order_type  Optional: 'gtc' (default), 'fok', 'gtd', or 'fak'");
    println!();
    println!("Environment Variables (set in .env file):");
    println!("  PRIVATE_KEY     Your EOA private key (0x prefixed) - wallet address derived from this");
    println!("  API_KEY         Polymarket API key (optional - will be derived if not provided)");
    println!("  API_SECRET      Polymarket API secret (optional)");
    println!("  API_PASSPHRASE  Polymarket API passphrase (optional)");
    println!();
    println!("Note: This binary uses EOA signature type (0). Your wallet address is derived");
    println!("from PRIVATE_KEY and used directly for trading (no proxy wallet needed).");
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
        _ => bail!("Invalid order type '{}'. Use 'gtc', 'fok', 'gtd', or 'fak'", s),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file
    dotenv::dotenv().ok();

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
    let price: f64 = args[2]
        .parse()
        .context("Failed to parse price as number")?;
    let size: f64 = args[3]
        .parse()
        .context("Failed to parse size as number")?;
    let side = parse_side(&args[4])?;
    let order_type = if args.len() > 5 {
        parse_order_type(&args[5])?
    } else {
        OrderType::GTC
    };

    // Validate inputs
    if price <= 0.0 || price >= 1.0 {
        bail!("Price must be between 0 and 1 (exclusive), got: {}", price);
    }
    if size <= 0.0 {
        bail!("Size must be positive, got: {}", size);
    }

    println!();
    println!("════════════════════════════════════════════════════════════════");
    println!("EOA ORDER PLACEMENT TEST");
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

    // Load credentials from environment
    let private_key = env::var("PRIVATE_KEY")
        .context("PRIVATE_KEY not found in environment. Check your .env file.")?;

    info!("Setting up authentication...");

    // Create auth manager
    let mut auth = PolymarketAuth::new(&private_key, POLYGON_CHAIN_ID)
        .context("Failed to create auth from private key")?;

    // For EOA, the wallet address derived from private key is both signer and maker
    let wallet_addr = auth.address().expect("EOA auth requires wallet address");

    println!("  Wallet:     {:?}", wallet_addr);
    println!("  Sig Type:   EOA (0)");

    // Create REST client
    let rest_client = RestClient::new(CLOB_URL);

    // Check if we have API credentials in env, otherwise derive them
    let api_key = env::var("API_KEY").ok();
    let api_secret = env::var("API_SECRET").ok();
    let api_passphrase = env::var("API_PASSPHRASE").ok();

    if let (Some(key), Some(secret), Some(passphrase)) = (api_key, api_secret, api_passphrase) {
        info!("Using API credentials from environment");
        auth.set_api_key(polymarket::infrastructure::client::clob::ApiCredentials {
            key,
            secret,
            passphrase,
        });
    } else {
        info!("Deriving API credentials from private key...");
        let creds = rest_client
            .get_or_create_api_creds(&auth)
            .await
            .context("Failed to get or create API credentials")?;
        auth.set_api_key(creds);
        info!("API credentials obtained successfully");
    }

    // Fetch neg_risk status for this token (affects EIP-712 domain)
    info!("Checking neg_risk status for token...");
    let neg_risk = rest_client
        .get_neg_risk(token_id)
        .await
        .unwrap_or_else(|e| {
            info!("Could not fetch neg_risk ({}), defaulting to false", e);
            false
        });
    println!("  Neg Risk:   {}", neg_risk);

    // Create EOA order builder - signer == maker, signature_type = 0
    let order_builder = OrderBuilder::new_eoa(wallet_addr, POLYGON_CHAIN_ID, neg_risk);

    println!();
    println!("════════════════════════════════════════════════════════════════");
    println!("PLACING ORDER...");
    println!("════════════════════════════════════════════════════════════════");
    println!();

    // Place the order
    let result = rest_client
        .place_signed_order(
            &auth,
            &order_builder,
            token_id,
            price,
            size,
            side,
            order_type,
            None, // default fee rate
        )
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
            if let Some(error) = &response.error_msg {
                if !error.is_empty() {
                    println!("  Error:      {}", error);
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
