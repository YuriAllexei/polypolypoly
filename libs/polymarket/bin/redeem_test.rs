use polymarket::infrastructure::client::{redeem_all, fetch_redeemable_positions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    dotenv::dotenv().ok();

    let proxy_wallet = std::env::var("PROXY_WALLET")?;

    println!("\nFetching redeemable positions for {}...\n", proxy_wallet);

    let positions = fetch_redeemable_positions(&proxy_wallet).await?;

    if positions.is_empty() {
        println!("No redeemable positions found.");
        return Ok(());
    }

    println!("Found {} redeemable position(s):\n", positions.len());
    for (i, p) in positions.iter().enumerate() {
        println!("{}. {} - {} ({} shares, ${:.2})",
            i + 1, p.title, p.outcome, p.size, p.current_value);
    }

    print!("\nProceed with redemption? (y/n): ");
    std::io::Write::flush(&mut std::io::stdout())?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    if input.trim().to_lowercase() != "y" {
        println!("Cancelled.");
        return Ok(());
    }

    println!("\nRedeeming...\n");

    let results = redeem_all().await?;

    let success = results.iter().filter(|r| r.tx_hash.is_some()).count();
    let failed = results.iter().filter(|r| r.error.is_some()).count();

    println!("\nResults:");
    for r in &results {
        if let Some(tx) = r.tx_hash {
            println!("  [OK] {} - https://polygonscan.com/tx/{:?}", r.title, tx);
        } else if let Some(err) = &r.error {
            println!("  [FAIL] {} - {}", r.title, err);
        }
    }

    println!("\nSummary: {} succeeded, {} failed", success, failed);

    Ok(())
}
