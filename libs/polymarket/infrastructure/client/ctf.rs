//! CTF (Conditional Token Framework) Operations
//!
//! Provides split and merge functionality for Polymarket outcome tokens.
//!
//! # Operations
//!
//! - **Split**: Convert USDC into YES + NO outcome tokens
//!   - 1 USDC → 1 YES + 1 NO token
//!   - Useful for market making or hedging
//!
//! - **Merge**: Convert YES + NO tokens back into USDC
//!   - 1 YES + 1 NO → 1 USDC
//!   - Useful for exiting positions or arbitrage when YES + NO < $1.00
//!
//! # Gas Price
//!
//! Gas prices are fetched dynamically from the Polygon network with a configurable
//! multiplier (default 1.2x) to ensure transactions don't get stuck during congestion.
//!
//! # Concurrency Warning
//!
//! **Important**: This module is NOT safe for concurrent Safe transactions.
//! Gnosis Safe uses sequential nonces - if multiple transactions are submitted
//! simultaneously, they may use the same nonce causing one to fail.
//! For concurrent use, implement external transaction queuing.
//!
//! # Usage
//!
//! ```rust,ignore
//! use polymarket::infrastructure::client::ctf::{CtfClient, split_via_safe, merge_via_safe};
//!
//! // Split 100 USDC into 100 YES + 100 NO tokens
//! let tx = split_via_safe(
//!     safe_address,
//!     condition_id,
//!     false, // neg_risk
//!     100_000_000, // 100 USDC (6 decimals)
//!     &wallet,
//!     POLYGON_RPC_URL,
//! ).await?;
//!
//! // Merge 50 YES + 50 NO tokens back into 50 USDC
//! let tx = merge_via_safe(
//!     safe_address,
//!     condition_id,
//!     false,
//!     50_000_000, // 50 USDC worth
//!     &wallet,
//!     POLYGON_RPC_URL,
//! ).await?;
//! ```

use ethers::prelude::*;
use ethers::contract::abigen;
use std::sync::Arc;
use thiserror::Error;
use tracing::{info, debug};

// Contract addresses on Polygon
pub const POLYGON_RPC_URL: &str = "https://polygon-rpc.com";
pub const POLYGON_CHAIN_ID: u64 = 137;
pub const CTF_CONTRACT: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
pub const NEG_RISK_CTF_CONTRACT: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";
pub const USDC_ADDRESS: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";

/// Gas limit for CTF operations (split/merge/approve)
/// Split/merge involve multiple token operations so needs higher limit
const GAS_LIMIT: u64 = 500_000;

/// Multiplier for gas price (1.2 = 20% above network estimate)
/// Increase this during high congestion periods
pub const GAS_PRICE_MULTIPLIER: f64 = 1.2;

/// Minimum gas price in gwei (floor to prevent too-low estimates)
const MIN_GAS_PRICE_GWEI: u64 = 30;

/// Maximum gas price in gwei (ceiling for high congestion periods)
const MAX_GAS_PRICE_GWEI: u64 = 1200;

/// USDC has 6 decimal places
pub const USDC_DECIMALS: u8 = 6;

// Generate contract bindings for CTF
abigen!(
    ConditionalTokens,
    r#"[
        function splitPosition(address collateralToken, bytes32 parentCollectionId, bytes32 conditionId, uint256[] calldata partition, uint256 amount) external
        function mergePositions(address collateralToken, bytes32 parentCollectionId, bytes32 conditionId, uint256[] calldata partition, uint256 amount) external
        function balanceOf(address account, uint256 id) external view returns (uint256)
        function getPositionId(address collateralToken, bytes32 collectionId) external pure returns (uint256)
        function getCollectionId(bytes32 parentCollectionId, bytes32 conditionId, uint256 indexSet) external view returns (bytes32)
    ]"#
);

// Generate contract bindings for ERC20 (USDC)
abigen!(
    ERC20,
    r#"[
        function approve(address spender, uint256 amount) external returns (bool)
        function allowance(address owner, address spender) external view returns (uint256)
        function balanceOf(address account) external view returns (uint256)
    ]"#
);

// Generate contract bindings for Gnosis Safe
abigen!(
    GnosisSafe,
    r#"[
        function execTransaction(address to, uint256 value, bytes calldata data, uint8 operation, uint256 safeTxGas, uint256 baseGas, uint256 gasPrice, address gasToken, address payable refundReceiver, bytes memory signatures) external payable returns (bool success)
        function nonce() external view returns (uint256)
    ]"#
);

// =============================================================================
// Error Types
// =============================================================================

#[derive(Error, Debug)]
pub enum CtfError {
    #[error("Provider error: {0}")]
    ProviderError(String),
    #[error("Contract error: {0}")]
    ContractError(String),
    #[error("Invalid condition ID: {0}")]
    InvalidConditionId(String),
    #[error("Transaction failed: {0}")]
    TransactionFailed(String),
    #[error("Insufficient balance: {0}")]
    InsufficientBalance(String),
    #[error("Approval failed: {0}")]
    ApprovalFailed(String),
}

pub type Result<T> = std::result::Result<T, CtfError>;

// =============================================================================
// Operation Types
// =============================================================================

/// Result of a split or merge operation
#[derive(Debug, Clone)]
pub struct CtfOperationResult {
    pub operation: CtfOperation,
    pub condition_id: String,
    pub amount: U256,
    pub neg_risk: bool,
    pub tx_hash: Option<TxHash>,
    pub error: Option<String>,
}

/// Type of CTF operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtfOperation {
    Split,
    Merge,
    Approve,
}

impl std::fmt::Display for CtfOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CtfOperation::Split => write!(f, "Split"),
            CtfOperation::Merge => write!(f, "Merge"),
            CtfOperation::Approve => write!(f, "Approve"),
        }
    }
}

// =============================================================================
// CTF Client
// =============================================================================

/// Client for CTF split/merge operations
pub struct CtfClient<M: Middleware> {
    ctf: ConditionalTokens<M>,
    neg_risk_ctf: ConditionalTokens<M>,
    usdc: ERC20<M>,
    usdc_address: Address,
    #[allow(dead_code)]
    provider: Arc<M>,
}

impl<M: Middleware + 'static> CtfClient<M> {
    /// Create a new CTF client
    pub fn new(provider: Arc<M>) -> Self {
        let ctf_address: Address = CTF_CONTRACT.parse().unwrap();
        let neg_risk_address: Address = NEG_RISK_CTF_CONTRACT.parse().unwrap();
        let usdc_address: Address = USDC_ADDRESS.parse().unwrap();

        Self {
            ctf: ConditionalTokens::new(ctf_address, provider.clone()),
            neg_risk_ctf: ConditionalTokens::new(neg_risk_address, provider.clone()),
            usdc: ERC20::new(usdc_address, provider.clone()),
            usdc_address,
            provider,
        }
    }

    /// Get the CTF contract address based on neg_risk flag
    pub fn ctf_address(&self, neg_risk: bool) -> Address {
        if neg_risk {
            NEG_RISK_CTF_CONTRACT.parse().unwrap()
        } else {
            CTF_CONTRACT.parse().unwrap()
        }
    }

    /// Check USDC allowance for CTF contract
    pub async fn check_allowance(&self, owner: Address, neg_risk: bool) -> Result<U256> {
        let spender = self.ctf_address(neg_risk);
        self.usdc
            .allowance(owner, spender)
            .call()
            .await
            .map_err(|e| CtfError::ContractError(e.to_string()))
    }

    /// Check USDC balance
    pub async fn check_usdc_balance(&self, account: Address) -> Result<U256> {
        self.usdc
            .balance_of(account)
            .call()
            .await
            .map_err(|e| CtfError::ContractError(e.to_string()))
    }

    /// Encode USDC approval call
    pub fn encode_approve_call(&self, neg_risk: bool, amount: U256) -> Result<(Address, Bytes)> {
        let spender = self.ctf_address(neg_risk);
        let call = self.usdc.approve(spender, amount);
        Ok((self.usdc_address, call.calldata().unwrap_or_default()))
    }

    /// Encode split position call
    ///
    /// Splits `amount` USDC into `amount` YES + `amount` NO tokens
    pub fn encode_split_call(&self, condition_id: &str, neg_risk: bool, amount: U256) -> Result<(Address, Bytes)> {
        let condition_id = parse_condition_id(condition_id)?;
        let contract = if neg_risk { &self.neg_risk_ctf } else { &self.ctf };

        // Binary market partition: [1, 2] represents YES (0b01) and NO (0b10)
        let partition = vec![U256::from(1), U256::from(2)];

        let call = contract.split_position(
            self.usdc_address,
            [0u8; 32], // parentCollectionId is always 0 for Polymarket
            condition_id,
            partition,
            amount,
        );

        Ok((self.ctf_address(neg_risk), call.calldata().unwrap_or_default()))
    }

    /// Encode merge positions call
    ///
    /// Merges `amount` YES + `amount` NO tokens into `amount` USDC
    pub fn encode_merge_call(&self, condition_id: &str, neg_risk: bool, amount: U256) -> Result<(Address, Bytes)> {
        let condition_id = parse_condition_id(condition_id)?;
        let contract = if neg_risk { &self.neg_risk_ctf } else { &self.ctf };

        // Binary market partition: [1, 2] represents YES (0b01) and NO (0b10)
        let partition = vec![U256::from(1), U256::from(2)];

        let call = contract.merge_positions(
            self.usdc_address,
            [0u8; 32], // parentCollectionId is always 0 for Polymarket
            condition_id,
            partition,
            amount,
        );

        Ok((self.ctf_address(neg_risk), call.calldata().unwrap_or_default()))
    }

    /// Get balance of a specific position token
    pub async fn get_position_balance(&self, account: Address, position_id: U256, neg_risk: bool) -> Result<U256> {
        let contract = if neg_risk { &self.neg_risk_ctf } else { &self.ctf };
        contract
            .balance_of(account, position_id)
            .call()
            .await
            .map_err(|e| CtfError::ContractError(e.to_string()))
    }
}

// =============================================================================
// Safe Transaction Execution
// =============================================================================

/// Execute a split operation via Gnosis Safe
///
/// Splits USDC into YES + NO outcome tokens.
/// Will automatically approve USDC if needed.
pub async fn split_via_safe(
    safe_address: Address,
    condition_id: &str,
    neg_risk: bool,
    amount: U256,
    wallet: &LocalWallet,
    rpc_url: &str,
) -> Result<TxHash> {
    let provider = Provider::<Http>::try_from(rpc_url)
        .map_err(|e| CtfError::ProviderError(e.to_string()))?;
    let provider = Arc::new(SignerMiddleware::new(provider, wallet.clone()));

    let client = CtfClient::new(provider.clone());

    // Check and handle approval if needed
    let allowance = client.check_allowance(safe_address, neg_risk).await?;
    if allowance < amount {
        info!("[CTF] Approving USDC for split (current allowance: {}, needed: {})", allowance, amount);
        approve_via_safe_internal(
            safe_address,
            neg_risk,
            U256::MAX, // Approve max to avoid repeated approvals
            wallet,
            &provider,
        ).await?;
        info!("[CTF] USDC approved");
    }

    // Check USDC balance
    let balance = client.check_usdc_balance(safe_address).await?;
    if balance < amount {
        return Err(CtfError::InsufficientBalance(format!(
            "USDC balance {} < required {}", balance, amount
        )));
    }

    // Execute split
    let (to, data) = client.encode_split_call(condition_id, neg_risk, amount)?;

    info!("[CTF] Splitting {} USDC for condition {}", amount, condition_id);
    execute_safe_tx(safe_address, to, data, wallet, &provider).await
}

/// Execute a merge operation via Gnosis Safe
///
/// Merges YES + NO outcome tokens back into USDC.
pub async fn merge_via_safe(
    safe_address: Address,
    condition_id: &str,
    neg_risk: bool,
    amount: U256,
    wallet: &LocalWallet,
    rpc_url: &str,
) -> Result<TxHash> {
    let provider = Provider::<Http>::try_from(rpc_url)
        .map_err(|e| CtfError::ProviderError(e.to_string()))?;
    let provider = Arc::new(SignerMiddleware::new(provider, wallet.clone()));

    let client = CtfClient::new(provider.clone());
    let (to, data) = client.encode_merge_call(condition_id, neg_risk, amount)?;

    info!("[CTF] Merging {} tokens for condition {}", amount, condition_id);
    execute_safe_tx(safe_address, to, data, wallet, &provider).await
}

/// Approve USDC spending for CTF contract via Gnosis Safe
pub async fn approve_via_safe(
    safe_address: Address,
    neg_risk: bool,
    amount: U256,
    wallet: &LocalWallet,
    rpc_url: &str,
) -> Result<TxHash> {
    let provider = Provider::<Http>::try_from(rpc_url)
        .map_err(|e| CtfError::ProviderError(e.to_string()))?;
    let provider = Arc::new(SignerMiddleware::new(provider, wallet.clone()));

    approve_via_safe_internal(safe_address, neg_risk, amount, wallet, &provider).await
}

/// Internal approval function (reused by split)
async fn approve_via_safe_internal<M: Middleware + 'static>(
    safe_address: Address,
    neg_risk: bool,
    amount: U256,
    wallet: &LocalWallet,
    provider: &Arc<M>,
) -> Result<TxHash> {
    let client = CtfClient::new(provider.clone());
    let (to, data) = client.encode_approve_call(neg_risk, amount)?;

    debug!("[CTF] Approving {} USDC for CTF contract", amount);
    execute_safe_tx(safe_address, to, data, wallet, provider).await
}

/// Fetch current gas price from the network and apply multiplier
///
/// Returns gas price in wei with safety bounds applied.
async fn get_dynamic_gas_price<M: Middleware + 'static>(provider: &Arc<M>) -> Result<U256> {
    // Fetch current gas price from network
    let network_gas_price = provider
        .get_gas_price()
        .await
        .map_err(|e| CtfError::ProviderError(format!("Failed to fetch gas price: {}", e)))?;

    // Convert to gwei for calculation
    let gas_price_gwei = network_gas_price.as_u64() / 1_000_000_000;

    // Apply multiplier
    let adjusted_gwei = (gas_price_gwei as f64 * GAS_PRICE_MULTIPLIER) as u64;

    // Apply bounds
    let final_gwei = adjusted_gwei.max(MIN_GAS_PRICE_GWEI).min(MAX_GAS_PRICE_GWEI);

    debug!(
        "[CTF] Gas price: network={}gwei, adjusted={}gwei, final={}gwei",
        gas_price_gwei, adjusted_gwei, final_gwei
    );

    Ok(U256::from(final_gwei) * U256::from(1_000_000_000u64))
}

/// Execute a transaction via Gnosis Safe
async fn execute_safe_tx<M: Middleware + 'static>(
    safe_address: Address,
    to: Address,
    data: Bytes,
    wallet: &LocalWallet,
    provider: &Arc<M>,
) -> Result<TxHash> {
    let safe = GnosisSafe::new(safe_address, provider.clone());
    let nonce = safe.nonce().call().await
        .map_err(|e| CtfError::ContractError(e.to_string()))?;

    let safe_tx_hash = compute_safe_tx_hash(
        safe_address, to, U256::zero(), data.clone(),
        0, U256::zero(), U256::zero(), U256::zero(),
        Address::zero(), Address::zero(), nonce, POLYGON_CHAIN_ID,
    );

    let signature = wallet.sign_hash(H256::from(safe_tx_hash))
        .map_err(|e| CtfError::ContractError(e.to_string()))?;

    // Fetch dynamic gas price from network
    let gas_price = get_dynamic_gas_price(provider).await?;

    let call = safe.exec_transaction(
        to, U256::zero(), data, 0,
        U256::zero(), U256::zero(), U256::zero(),
        Address::zero(), Address::zero(), signature.to_vec().into(),
    )
    .gas(U256::from(GAS_LIMIT))
    .gas_price(gas_price);

    let pending_tx = call.send().await
        .map_err(|e| CtfError::ContractError(e.to_string()))?;

    let tx_hash = pending_tx.tx_hash();
    debug!("[CTF] Transaction sent: {:?} (gas_price: {} gwei)", tx_hash, gas_price / U256::from(1_000_000_000u64));

    let receipt = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        pending_tx,
    )
    .await
    .map_err(|_| CtfError::TransactionFailed(format!("Timeout. TX: {:?}", tx_hash)))?
    .map_err(|e| CtfError::TransactionFailed(e.to_string()))?
    .ok_or_else(|| CtfError::TransactionFailed("No receipt".to_string()))?;

    if receipt.status == Some(U64::from(1)) {
        info!("[CTF] Transaction confirmed: {:?}", tx_hash);
        Ok(tx_hash)
    } else {
        Err(CtfError::TransactionFailed("Transaction reverted".to_string()))
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Parse a condition ID string to bytes32
fn parse_condition_id(condition_id: &str) -> Result<[u8; 32]> {
    let hex_str = condition_id.trim_start_matches("0x");
    if hex_str.len() != 64 {
        return Err(CtfError::InvalidConditionId(format!(
            "Expected 64 hex chars, got {}", hex_str.len()
        )));
    }
    let bytes = hex::decode(hex_str)
        .map_err(|e| CtfError::InvalidConditionId(e.to_string()))?;
    let mut result = [0u8; 32];
    result.copy_from_slice(&bytes);
    Ok(result)
}

/// Compute Gnosis Safe transaction hash for signing
fn compute_safe_tx_hash(
    safe: Address, to: Address, value: U256, data: Bytes,
    operation: u8, safe_tx_gas: U256, base_gas: U256, gas_price: U256,
    gas_token: Address, refund_receiver: Address, nonce: U256, chain_id: u64,
) -> [u8; 32] {
    use ethers::utils::keccak256;

    let domain_type_hash = keccak256(b"EIP712Domain(uint256 chainId,address verifyingContract)");
    let mut domain_data = Vec::with_capacity(96);
    domain_data.extend_from_slice(&domain_type_hash);
    domain_data.extend_from_slice(&[0u8; 24]);
    domain_data.extend_from_slice(&chain_id.to_be_bytes());
    domain_data.extend_from_slice(&[0u8; 12]);
    domain_data.extend_from_slice(safe.as_bytes());
    let domain_separator = keccak256(&domain_data);

    let safe_tx_type_hash = keccak256(
        b"SafeTx(address to,uint256 value,bytes data,uint8 operation,uint256 safeTxGas,uint256 baseGas,uint256 gasPrice,address gasToken,address refundReceiver,uint256 nonce)"
    );

    let mut struct_data = Vec::with_capacity(384);
    struct_data.extend_from_slice(&safe_tx_type_hash);
    struct_data.extend_from_slice(&[0u8; 12]);
    struct_data.extend_from_slice(to.as_bytes());
    struct_data.extend_from_slice(&u256_to_bytes32(value));
    struct_data.extend_from_slice(&keccak256(&data));
    struct_data.extend_from_slice(&[0u8; 31]);
    struct_data.push(operation);
    struct_data.extend_from_slice(&u256_to_bytes32(safe_tx_gas));
    struct_data.extend_from_slice(&u256_to_bytes32(base_gas));
    struct_data.extend_from_slice(&u256_to_bytes32(gas_price));
    struct_data.extend_from_slice(&[0u8; 12]);
    struct_data.extend_from_slice(gas_token.as_bytes());
    struct_data.extend_from_slice(&[0u8; 12]);
    struct_data.extend_from_slice(refund_receiver.as_bytes());
    struct_data.extend_from_slice(&u256_to_bytes32(nonce));
    let struct_hash = keccak256(&struct_data);

    let mut final_data = Vec::with_capacity(66);
    final_data.push(0x19);
    final_data.push(0x01);
    final_data.extend_from_slice(&domain_separator);
    final_data.extend_from_slice(&struct_hash);

    keccak256(&final_data)
}

fn u256_to_bytes32(value: U256) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    value.to_big_endian(&mut bytes);
    bytes
}

/// Convert USDC amount (human readable) to raw units
///
/// Example: `usdc_to_raw(100.0)` returns 100_000_000 (100 USDC with 6 decimals)
pub fn usdc_to_raw(amount: f64) -> U256 {
    let raw = (amount * 10f64.powi(USDC_DECIMALS as i32)) as u64;
    U256::from(raw)
}

/// Convert raw USDC units to human readable
///
/// Example: `usdc_from_raw(100_000_000)` returns 100.0
pub fn usdc_from_raw(raw: U256) -> f64 {
    let raw_u64 = raw.as_u64();
    raw_u64 as f64 / 10f64.powi(USDC_DECIMALS as i32)
}

// =============================================================================
// Convenience Functions (load from env)
// =============================================================================

fn load_private_key() -> Result<String> {
    dotenv::dotenv().ok();
    std::env::var("PRIVATE_KEY")
        .map_err(|_| CtfError::ProviderError("PRIVATE_KEY not set".to_string()))
}

fn load_proxy_wallet() -> Result<String> {
    dotenv::dotenv().ok();
    std::env::var("PROXY_WALLET")
        .map_err(|_| CtfError::ProviderError("PROXY_WALLET not set".to_string()))
}

/// Split USDC into outcome tokens using env credentials
pub async fn split(condition_id: &str, neg_risk: bool, amount: U256) -> Result<TxHash> {
    let private_key = load_private_key()?;
    let proxy_wallet = load_proxy_wallet()?;

    let wallet: LocalWallet = private_key.trim_start_matches("0x")
        .parse()
        .map_err(|e: WalletError| CtfError::ProviderError(e.to_string()))?;
    let wallet = wallet.with_chain_id(POLYGON_CHAIN_ID);

    let safe_address: Address = proxy_wallet
        .parse()
        .map_err(|_| CtfError::ProviderError("Invalid proxy wallet".to_string()))?;

    split_via_safe(safe_address, condition_id, neg_risk, amount, &wallet, POLYGON_RPC_URL).await
}

/// Merge outcome tokens back into USDC using env credentials
pub async fn merge(condition_id: &str, neg_risk: bool, amount: U256) -> Result<TxHash> {
    let private_key = load_private_key()?;
    let proxy_wallet = load_proxy_wallet()?;

    let wallet: LocalWallet = private_key.trim_start_matches("0x")
        .parse()
        .map_err(|e: WalletError| CtfError::ProviderError(e.to_string()))?;
    let wallet = wallet.with_chain_id(POLYGON_CHAIN_ID);

    let safe_address: Address = proxy_wallet
        .parse()
        .map_err(|_| CtfError::ProviderError("Invalid proxy wallet".to_string()))?;

    merge_via_safe(safe_address, condition_id, neg_risk, amount, &wallet, POLYGON_RPC_URL).await
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_condition_id() {
        let valid = "0xabcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
        assert!(parse_condition_id(valid).is_ok());
        assert!(parse_condition_id(&valid[2..]).is_ok()); // Without 0x prefix
        assert!(parse_condition_id("invalid").is_err());
        assert!(parse_condition_id("0x1234").is_err()); // Too short
    }

    #[test]
    fn test_contract_addresses() {
        assert!(CTF_CONTRACT.parse::<Address>().is_ok());
        assert!(NEG_RISK_CTF_CONTRACT.parse::<Address>().is_ok());
        assert!(USDC_ADDRESS.parse::<Address>().is_ok());
    }

    #[test]
    fn test_usdc_conversion() {
        // 100 USDC
        let raw = usdc_to_raw(100.0);
        assert_eq!(raw, U256::from(100_000_000u64));

        // Convert back
        let human = usdc_from_raw(raw);
        assert!((human - 100.0).abs() < 0.000001);

        // 0.5 USDC
        let raw = usdc_to_raw(0.5);
        assert_eq!(raw, U256::from(500_000u64));
    }

    #[test]
    fn test_operation_display() {
        assert_eq!(format!("{}", CtfOperation::Split), "Split");
        assert_eq!(format!("{}", CtfOperation::Merge), "Merge");
        assert_eq!(format!("{}", CtfOperation::Approve), "Approve");
    }

    #[test]
    fn test_ctf_address_selection() {
        let provider = Arc::new(Provider::<Http>::try_from("https://polygon-rpc.com").unwrap());
        let client = CtfClient::new(provider);

        let normal_addr = client.ctf_address(false);
        let neg_risk_addr = client.ctf_address(true);

        assert_eq!(normal_addr, CTF_CONTRACT.parse::<Address>().unwrap());
        assert_eq!(neg_risk_addr, NEG_RISK_CTF_CONTRACT.parse::<Address>().unwrap());
        assert_ne!(normal_addr, neg_risk_addr);
    }

    #[test]
    fn test_encode_split_call() {
        let provider = Arc::new(Provider::<Http>::try_from("https://polygon-rpc.com").unwrap());
        let client = CtfClient::new(provider);

        let condition_id = "0xabcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
        let amount = U256::from(100_000_000u64); // 100 USDC

        let result = client.encode_split_call(condition_id, false, amount);
        assert!(result.is_ok());

        let (to, data) = result.unwrap();
        assert_eq!(to, CTF_CONTRACT.parse::<Address>().unwrap());
        assert!(!data.is_empty());
    }

    #[test]
    fn test_encode_merge_call() {
        let provider = Arc::new(Provider::<Http>::try_from("https://polygon-rpc.com").unwrap());
        let client = CtfClient::new(provider);

        let condition_id = "0xabcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
        let amount = U256::from(50_000_000u64); // 50 USDC

        let result = client.encode_merge_call(condition_id, false, amount);
        assert!(result.is_ok());

        let (to, data) = result.unwrap();
        assert_eq!(to, CTF_CONTRACT.parse::<Address>().unwrap());
        assert!(!data.is_empty());
    }

    #[test]
    fn test_encode_approve_call() {
        let provider = Arc::new(Provider::<Http>::try_from("https://polygon-rpc.com").unwrap());
        let client = CtfClient::new(provider);

        let result = client.encode_approve_call(false, U256::MAX);
        assert!(result.is_ok());

        let (to, data) = result.unwrap();
        assert_eq!(to, USDC_ADDRESS.parse::<Address>().unwrap());
        assert!(!data.is_empty());
    }
}
