//! Test binary for order cancellation
//!
//! Usage:
//!   cargo run --bin test_cancel -- single <order_id>
//!   cargo run --bin test_cancel -- all
//!   cargo run --bin test_cancel -- list

use anyhow::Result;
use polymarket::infrastructure::client::clob::TradingClient;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();
    let client = TradingClient::from_env().await?;

    match args.get(1).map(|s| s.as_str()) {
        Some("single") => {
            let order_id = args.get(2).ok_or(anyhow::anyhow!("Missing order_id"))?;
            println!("Cancelling order: {}", order_id);

            let result = client.cancel_order(order_id).await?;
            println!("✅ Cancelled: {:?}", result.canceled);
            if !result.not_canceled.is_empty() {
                println!("❌ Failed: {:?}", result.not_canceled);
            }
        }

        Some("all") => {
            println!("Cancelling ALL open orders...");

            let result = client.cancel_all().await?;
            println!("✅ Cancelled {} orders: {:?}", result.canceled.len(), result.canceled);
            if !result.not_canceled.is_empty() {
                println!("❌ Failed: {:?}", result.not_canceled);
            }
        }

        Some("list") => {
            println!("Fetching open orders...");

            let orders = client.get_orders(None).await?;
            if orders.is_empty() {
                println!("No open orders found.");
            } else {
                println!("Found {} open orders:", orders.len());
                for order in &orders {
                    if let Some(id) = order.get("id").and_then(|v| v.as_str()) {
                        let side = order.get("side").and_then(|v| v.as_str()).unwrap_or("?");
                        let price = order.get("price").and_then(|v| v.as_str()).unwrap_or("?");
                        let size = order.get("original_size").and_then(|v| v.as_str()).unwrap_or("?");
                        println!("  {} | {} @ {} | size: {}", id, side, price, size);
                    }
                }
            }
        }

        _ => {
            println!("Order Cancellation Test Tool");
            println!();
            println!("Usage:");
            println!("  cargo run --bin test_cancel -- list              # List open orders");
            println!("  cargo run --bin test_cancel -- single <order_id> # Cancel one order");
            println!("  cargo run --bin test_cancel -- all               # Cancel all orders");
        }
    }

    Ok(())
}
