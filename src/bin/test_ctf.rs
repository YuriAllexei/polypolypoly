//! Test binary for CTF Split/Merge Operations
//!
//! Test splitting USDC into YES+NO tokens and merging them back.
//!
//! Usage:
//!   cargo run --bin test_ctf positions              # List your positions with condition IDs
//!   cargo run --bin test_ctf split <condition_id> <amount_usdc> [neg_risk]
//!   cargo run --bin test_ctf merge <condition_id> <amount_usdc> [neg_risk]
//!   cargo run --bin test_ctf check
//!
//! Examples:
//!   cargo run --bin test_ctf positions
//!   cargo run --bin test_ctf split 0xabc123...def 10.0
//!   cargo run --bin test_ctf merge 0xabc123...def 10.0 neg_risk
//!   cargo run --bin test_ctf check

use anyhow::Result;
use ethers::prelude::*;
use polymarket::infrastructure::{
    split_via_safe, merge_via_safe, usdc_to_raw, usdc_from_raw,
    CtfClient,
};
use polymarket::infrastructure::client::data::DataApiClient;
use std::env;
use std::sync::Arc;

const POLYGON_RPC_URL: &str = "https://polygon-rpc.com";
const POLYGON_CHAIN_ID: u64 = 137;

fn print_usage() {
    println!("CTF Split/Merge Test Tool");
    println!();
    println!("Usage:");
    println!("  cargo run --bin test_ctf positions                              # List positions");
    println!("  cargo run --bin test_ctf split <condition_id> <amount> [neg_risk]");
    println!("  cargo run --bin test_ctf merge <condition_id> <amount> [neg_risk]");
    println!("  cargo run --bin test_ctf check                                  # Check balances");
    println!();
    println!("Examples:");
    println!("  cargo run --bin test_ctf positions");
    println!("  cargo run --bin test_ctf split 0xabc123... 10.0");
    println!("  cargo run --bin test_ctf merge 0xabc123... 10.0");
    println!("  cargo run --bin test_ctf split 0xabc123... 10.0 neg_risk");
    println!();
    println!("Arguments:");
    println!("  condition_id  - The market's condition ID (64 hex chars with 0x prefix)");
    println!("  amount        - Amount in USDC (e.g., 10.0 for $10)");
    println!("  neg_risk      - Optional: add 'neg_risk' for negative risk markets");
}

fn load_env() -> Result<(LocalWallet, Address, String)> {
    dotenv::dotenv().ok();

    let private_key = std::env::var("PRIVATE_KEY")
        .map_err(|_| anyhow::anyhow!("PRIVATE_KEY not set in .env"))?;
    let proxy_wallet = std::env::var("PROXY_WALLET")
        .map_err(|_| anyhow::anyhow!("PROXY_WALLET not set in .env"))?;

    let wallet: LocalWallet = private_key.trim_start_matches("0x")
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid private key: {}", e))?;
    let wallet = wallet.with_chain_id(POLYGON_CHAIN_ID);

    let safe_address: Address = proxy_wallet
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid proxy wallet address"))?;

    Ok((wallet, safe_address, proxy_wallet))
}

async fn do_positions() -> Result<()> {
    println!("════════════════════════════════════════════════════════════════════════════");
    println!("  YOUR POSITIONS");
    println!("════════════════════════════════════════════════════════════════════════════");
    println!();

    let (_, _, proxy_wallet) = load_env()?;
    println!("Fetching positions for {}...", proxy_wallet);
    println!();

    let client = DataApiClient::new();
    let positions = client.get_positions(&proxy_wallet, None).await
        .map_err(|e| anyhow::anyhow!("Failed to fetch positions: {}", e))?;

    if positions.is_empty() {
        println!("No positions found.");
        return Ok(());
    }

    // Group positions by condition_id to show mergeable pairs
    use std::collections::HashMap;
    let mut by_condition: HashMap<String, Vec<_>> = HashMap::new();
    for pos in &positions {
        by_condition.entry(pos.condition_id.clone()).or_default().push(pos);
    }

    println!("Found {} positions across {} markets:", positions.len(), by_condition.len());
    println!();

    for (condition_id, market_positions) in by_condition.iter() {
        let first = &market_positions[0];
        let neg_risk_str = if first.negative_risk { " [NEG_RISK]" } else { "" };

        println!("────────────────────────────────────────────────────────────────────────────");
        println!("Market: {}", first.title);
        println!("Condition ID: {}{}", condition_id, neg_risk_str);

        // Check if mergeable (both YES and NO positions)
        let has_yes = market_positions.iter().any(|p| p.outcome_index == 0);
        let has_no = market_positions.iter().any(|p| p.outcome_index == 1);
        let mergeable = has_yes && has_no;

        if mergeable {
            println!("Status: \x1B[32mMERGEABLE\x1B[0m (have both YES and NO)");
        } else if first.redeemable {
            println!("Status: \x1B[33mREDEEMABLE\x1B[0m (market resolved)");
        } else {
            println!("Status: Active");
        }

        for pos in market_positions {
            let side = if pos.outcome_index == 0 { "YES" } else { "NO" };
            println!("  {} : {:.2} shares @ ${:.4} avg (value: ${:.2})",
                     side, pos.size, pos.avg_price, pos.current_value);
        }

        if mergeable {
            let yes_size = market_positions.iter()
                .filter(|p| p.outcome_index == 0)
                .map(|p| p.size)
                .sum::<f64>();
            let no_size = market_positions.iter()
                .filter(|p| p.outcome_index == 1)
                .map(|p| p.size)
                .sum::<f64>();
            let max_merge = yes_size.min(no_size);
            println!("  -> Can merge up to: {:.2} tokens", max_merge);
            println!("  -> Command: cargo run --bin test_ctf merge {} {:.2}{}",
                     condition_id, max_merge,
                     if first.negative_risk { " neg_risk" } else { "" });
        }
        println!();
    }

    println!("════════════════════════════════════════════════════════════════════════════");
    println!();
    println!("To split USDC into tokens for a market:");
    println!("  cargo run --bin test_ctf split <condition_id> <amount>");
    println!();
    println!("To merge tokens back into USDC:");
    println!("  cargo run --bin test_ctf merge <condition_id> <amount>");

    Ok(())
}

async fn do_split(condition_id: &str, amount: f64, neg_risk: bool) -> Result<()> {
    println!("════════════════════════════════════════════════════════════════");
    println!("  CTF SPLIT OPERATION");
    println!("════════════════════════════════════════════════════════════════");
    println!("  Condition ID: {}", condition_id);
    println!("  Amount: {} USDC", amount);
    println!("  Neg Risk: {}", neg_risk);
    println!("  Result: {} YES + {} NO tokens", amount, amount);
    println!("════════════════════════════════════════════════════════════════");
    println!();

    let (wallet, safe_address, _) = load_env()?;
    let raw_amount = usdc_to_raw(amount);

    println!("Executing split via Safe {}...", safe_address);
    println!();

    match split_via_safe(
        safe_address,
        condition_id,
        neg_risk,
        raw_amount,
        &wallet,
        POLYGON_RPC_URL,
    ).await {
        Ok(tx_hash) => {
            println!("Split successful!");
            println!("TX Hash: {:?}", tx_hash);
            println!("View on Polygonscan: https://polygonscan.com/tx/{:?}", tx_hash);
            Ok(())
        }
        Err(e) => {
            eprintln!("Split failed: {}", e);
            Err(e.into())
        }
    }
}

async fn do_merge(condition_id: &str, amount: f64, neg_risk: bool) -> Result<()> {
    println!("════════════════════════════════════════════════════════════════");
    println!("  CTF MERGE OPERATION");
    println!("════════════════════════════════════════════════════════════════");
    println!("  Condition ID: {}", condition_id);
    println!("  Amount: {} YES + {} NO tokens", amount, amount);
    println!("  Neg Risk: {}", neg_risk);
    println!("  Result: {} USDC", amount);
    println!("════════════════════════════════════════════════════════════════");
    println!();

    let (wallet, safe_address, _) = load_env()?;
    let raw_amount = usdc_to_raw(amount);

    println!("Executing merge via Safe {}...", safe_address);
    println!();

    match merge_via_safe(
        safe_address,
        condition_id,
        neg_risk,
        raw_amount,
        &wallet,
        POLYGON_RPC_URL,
    ).await {
        Ok(tx_hash) => {
            println!("Merge successful!");
            println!("TX Hash: {:?}", tx_hash);
            println!("View on Polygonscan: https://polygonscan.com/tx/{:?}", tx_hash);
            Ok(())
        }
        Err(e) => {
            eprintln!("Merge failed: {}", e);
            Err(e.into())
        }
    }
}

async fn do_check() -> Result<()> {
    println!("════════════════════════════════════════════════════════════════");
    println!("  CTF BALANCE CHECK");
    println!("════════════════════════════════════════════════════════════════");
    println!();

    let (_, safe_address, _) = load_env()?;

    println!("Safe Address: {}", safe_address);
    println!();

    let provider = Provider::<Http>::try_from(POLYGON_RPC_URL)?;
    let provider = Arc::new(provider);
    let client = CtfClient::new(provider);

    // Check USDC balance
    let usdc_balance = client.check_usdc_balance(safe_address).await?;
    println!("USDC Balance: ${:.6} ({} raw)", usdc_from_raw(usdc_balance), usdc_balance);

    // Check allowances for both CTF contracts
    let allowance_normal = client.check_allowance(safe_address, false).await?;
    let allowance_neg_risk = client.check_allowance(safe_address, true).await?;

    println!();
    println!("USDC Allowance (Normal CTF): {}",
             if allowance_normal == U256::MAX { "MAX (unlimited)".to_string() }
             else { format!("${:.6}", usdc_from_raw(allowance_normal)) });

    println!("USDC Allowance (NegRisk CTF): {}",
             if allowance_neg_risk == U256::MAX { "MAX (unlimited)".to_string() }
             else { format!("${:.6}", usdc_from_raw(allowance_neg_risk)) });

    println!();
    println!("════════════════════════════════════════════════════════════════");

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    match args[1].as_str() {
        "positions" | "pos" | "p" => {
            do_positions().await?;
        }
        "split" => {
            if args.len() < 4 {
                eprintln!("Error: split requires <condition_id> and <amount>");
                print_usage();
                return Ok(());
            }
            let condition_id = &args[2];
            let amount: f64 = args[3].parse()
                .map_err(|_| anyhow::anyhow!("Invalid amount: {}", args[3]))?;
            let neg_risk = args.get(4).map(|s| s == "neg_risk").unwrap_or(false);

            do_split(condition_id, amount, neg_risk).await?;
        }
        "merge" => {
            if args.len() < 4 {
                eprintln!("Error: merge requires <condition_id> and <amount>");
                print_usage();
                return Ok(());
            }
            let condition_id = &args[2];
            let amount: f64 = args[3].parse()
                .map_err(|_| anyhow::anyhow!("Invalid amount: {}", args[3]))?;
            let neg_risk = args.get(4).map(|s| s == "neg_risk").unwrap_or(false);

            do_merge(condition_id, amount, neg_risk).await?;
        }
        "check" => {
            do_check().await?;
        }
        "help" | "--help" | "-h" => {
            print_usage();
        }
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            print_usage();
        }
    }

    Ok(())
}
