//! Position Manager
//!
//! Automatically redeems resolved Polymarket positions.
//! Checks every 60 seconds for redeemable positions.

use crate::infrastructure::client::redeem::{
    fetch_redeemable_positions, redeem_via_safe, POLYGON_CHAIN_ID, POLYGON_RPC_URL,
};
use ethers::prelude::*;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

pub struct PositionManager {
    proxy_wallet: Address,
    proxy_wallet_str: String,
    wallet: LocalWallet,
    task_handle: Option<JoinHandle<()>>,
}

impl PositionManager {
    /// Create from environment variables (PRIVATE_KEY, PROXY_WALLET)
    pub fn from_env() -> anyhow::Result<Self> {
        dotenv::dotenv().ok();

        let private_key =
            std::env::var("PRIVATE_KEY").map_err(|_| anyhow::anyhow!("PRIVATE_KEY not set"))?;
        let proxy_wallet_str =
            std::env::var("PROXY_WALLET").map_err(|_| anyhow::anyhow!("PROXY_WALLET not set"))?;

        let proxy_wallet: Address = proxy_wallet_str.parse()?;
        let wallet: LocalWallet = private_key
            .trim_start_matches("0x")
            .parse::<LocalWallet>()?
            .with_chain_id(POLYGON_CHAIN_ID);

        Ok(Self {
            proxy_wallet,
            proxy_wallet_str,
            wallet,
            task_handle: None,
        })
    }

    pub fn start(&mut self, shutdown_flag: Arc<AtomicBool>) {
        let proxy_wallet = self.proxy_wallet;
        let proxy_wallet_str = self.proxy_wallet_str.clone();
        let wallet = self.wallet.clone();

        info!("PositionManager started");

        let handle = tokio::spawn(async move {
            while shutdown_flag.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_secs(60)).await;

                if !shutdown_flag.load(Ordering::Acquire) {
                    break;
                }

                match fetch_redeemable_positions(&proxy_wallet_str).await {
                    Ok(positions) if !positions.is_empty() => {
                        info!(
                            "PositionManager: Found {} redeemable position(s)",
                            positions.len()
                        );

                        let mut seen = HashSet::new();
                        for position in positions {
                            if !seen.insert(position.condition_id.clone()) {
                                continue;
                            }

                            match redeem_via_safe(
                                proxy_wallet,
                                &position.condition_id,
                                position.negative_risk,
                                &wallet,
                                POLYGON_RPC_URL,
                            )
                            .await
                            {
                                Ok(tx_hash) => {
                                    info!(
                                        "PositionManager: Redeemed {} - TX: {:?}",
                                        position.title, tx_hash
                                    );
                                }
                                Err(e) => {
                                    warn!(
                                        "PositionManager: Failed to redeem {} - {}",
                                        position.title, e
                                    );
                                }
                            }

                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    }
                    Ok(_) => {
                        debug!("PositionManager: No redeemable positions");
                    }
                    Err(e) => {
                        warn!("PositionManager: Failed to fetch positions: {}", e);
                    }
                }
            }
            info!("PositionManager: Stopped");
        });

        self.task_handle = Some(handle);
    }

    pub async fn stop(&mut self) {
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
            let _ = handle.await;
            info!("PositionManager stopped");
        }
    }
}
