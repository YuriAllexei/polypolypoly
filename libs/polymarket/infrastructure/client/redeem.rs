//! On-chain redemption for resolved Polymarket positions via Gnosis Safe.

use ethers::prelude::*;
use ethers::contract::abigen;
use std::sync::Arc;
use thiserror::Error;
use tracing::{info, warn};

use super::data::{DataApiClient, Position};

pub const POLYGON_RPC_URL: &str = "https://polygon-rpc.com";
pub const POLYGON_CHAIN_ID: u64 = 137;
pub const CTF_CONTRACT: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
pub const NEG_RISK_CTF_CONTRACT: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";
pub const USDC_ADDRESS: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";

const GAS_PRICE_GWEI: u64 = 500;

abigen!(
    ConditionalTokens,
    r#"[
        function redeemPositions(address collateralToken, bytes32 parentCollectionId, bytes32 conditionId, uint256[] calldata indexSets) external
        function balanceOf(address account, uint256 id) external view returns (uint256)
        function payoutNumerators(bytes32 conditionId, uint256 index) external view returns (uint256)
        function payoutDenominator(bytes32 conditionId) external view returns (uint256)
    ]"#
);

abigen!(
    GnosisSafe,
    r#"[
        function execTransaction(address to, uint256 value, bytes calldata data, uint8 operation, uint256 safeTxGas, uint256 baseGas, uint256 gasPrice, address gasToken, address payable refundReceiver, bytes memory signatures) external payable returns (bool success)
        function nonce() external view returns (uint256)
    ]"#
);

#[derive(Error, Debug)]
pub enum RedeemError {
    #[error("Provider error: {0}")]
    ProviderError(String),
    #[error("Contract error: {0}")]
    ContractError(String),
    #[error("Invalid condition ID: {0}")]
    InvalidConditionId(String),
    #[error("Transaction failed: {0}")]
    TransactionFailed(String),
}

pub type Result<T> = std::result::Result<T, RedeemError>;

#[derive(Debug)]
pub struct RedemptionResult {
    pub condition_id: String,
    pub title: String,
    pub outcome: String,
    pub size: f64,
    pub neg_risk: bool,
    pub tx_hash: Option<TxHash>,
    pub error: Option<String>,
}

pub struct RedeemClient<M: Middleware> {
    ctf: ConditionalTokens<M>,
    neg_risk_ctf: ConditionalTokens<M>,
    usdc: Address,
}

impl<M: Middleware + 'static> RedeemClient<M> {
    pub fn new(provider: Arc<M>) -> Self {
        let ctf_address: Address = CTF_CONTRACT.parse().unwrap();
        let neg_risk_address: Address = NEG_RISK_CTF_CONTRACT.parse().unwrap();
        let usdc: Address = USDC_ADDRESS.parse().unwrap();

        Self {
            ctf: ConditionalTokens::new(ctf_address, provider.clone()),
            neg_risk_ctf: ConditionalTokens::new(neg_risk_address, provider),
            usdc,
        }
    }

    pub fn encode_redeem_call(&self, condition_id: &str, neg_risk: bool) -> Result<(Address, Bytes)> {
        let condition_id = parse_condition_id(condition_id)?;
        let contract = if neg_risk { &self.neg_risk_ctf } else { &self.ctf };
        let call = contract.redeem_positions(
            self.usdc,
            [0u8; 32],
            condition_id,
            vec![U256::from(1), U256::from(2)],
        );

        let ctf_address: Address = if neg_risk {
            NEG_RISK_CTF_CONTRACT
        } else {
            CTF_CONTRACT
        }.parse().unwrap();

        Ok((ctf_address, call.calldata().unwrap_or_default()))
    }

    pub async fn is_resolved(&self, condition_id: &str, neg_risk: bool) -> Result<bool> {
        let condition_id = parse_condition_id(condition_id)?;
        let contract = if neg_risk { &self.neg_risk_ctf } else { &self.ctf };
        let denominator = contract
            .payout_denominator(condition_id)
            .call()
            .await
            .map_err(|e| RedeemError::ContractError(e.to_string()))?;
        Ok(denominator > U256::zero())
    }
}

pub async fn redeem_via_safe(
    safe_address: Address,
    condition_id: &str,
    neg_risk: bool,
    wallet: &LocalWallet,
    rpc_url: &str,
) -> Result<TxHash> {
    let provider = Provider::<Http>::try_from(rpc_url)
        .map_err(|e| RedeemError::ProviderError(e.to_string()))?;
    let provider = Arc::new(SignerMiddleware::new(provider, wallet.clone()));

    let client = RedeemClient::new(provider.clone());
    let (ctf_address, call_data) = client.encode_redeem_call(condition_id, neg_risk)?;

    let safe = GnosisSafe::new(safe_address, provider);
    let nonce = safe.nonce().call().await
        .map_err(|e| RedeemError::ContractError(e.to_string()))?;

    let safe_tx_hash = compute_safe_tx_hash(
        safe_address, ctf_address, U256::zero(), call_data.clone(),
        0, U256::zero(), U256::zero(), U256::zero(),
        Address::zero(), Address::zero(), nonce, POLYGON_CHAIN_ID,
    );

    let signature = wallet.sign_hash(H256::from(safe_tx_hash))
        .map_err(|e| RedeemError::ContractError(e.to_string()))?;

    let call = safe.exec_transaction(
        ctf_address, U256::zero(), call_data, 0,
        U256::zero(), U256::zero(), U256::zero(),
        Address::zero(), Address::zero(), signature.to_vec().into(),
    ).gas_price(U256::from(GAS_PRICE_GWEI * 1_000_000_000));

    let pending_tx = call.send().await
        .map_err(|e| RedeemError::ContractError(e.to_string()))?;

    let tx_hash = pending_tx.tx_hash();

    let receipt = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        pending_tx,
    )
    .await
    .map_err(|_| RedeemError::TransactionFailed(format!("Timeout. TX: {:?}", tx_hash)))?
    .map_err(|e| RedeemError::TransactionFailed(e.to_string()))?
    .ok_or_else(|| RedeemError::TransactionFailed("No receipt".to_string()))?;

    if receipt.status == Some(U64::from(1)) {
        Ok(tx_hash)
    } else {
        Err(RedeemError::TransactionFailed("Transaction reverted".to_string()))
    }
}

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

fn parse_condition_id(condition_id: &str) -> Result<[u8; 32]> {
    let hex_str = condition_id.trim_start_matches("0x");
    if hex_str.len() != 64 {
        return Err(RedeemError::InvalidConditionId(format!(
            "Expected 64 hex chars, got {}", hex_str.len()
        )));
    }
    let bytes = hex::decode(hex_str)
        .map_err(|e| RedeemError::InvalidConditionId(e.to_string()))?;
    let mut result = [0u8; 32];
    result.copy_from_slice(&bytes);
    Ok(result)
}

fn load_private_key() -> Result<String> {
    dotenv::dotenv().ok();
    std::env::var("PRIVATE_KEY")
        .map_err(|_| RedeemError::ProviderError("PRIVATE_KEY not set".to_string()))
}

fn load_proxy_wallet() -> Result<String> {
    dotenv::dotenv().ok();
    std::env::var("PROXY_WALLET")
        .map_err(|_| RedeemError::ProviderError("PROXY_WALLET not set".to_string()))
}

pub async fn fetch_redeemable_positions(proxy_wallet: &str) -> Result<Vec<Position>> {
    DataApiClient::new()
        .get_redeemable_positions(proxy_wallet)
        .await
        .map_err(|e| RedeemError::ProviderError(e.to_string()))
}

pub async fn redeem_all_positions(proxy_wallet: &str) -> Result<Vec<RedemptionResult>> {
    let private_key = load_private_key()?;
    let wallet: LocalWallet = private_key.trim_start_matches("0x")
        .parse()
        .map_err(|e: WalletError| RedeemError::ProviderError(e.to_string()))?;
    let wallet = wallet.with_chain_id(POLYGON_CHAIN_ID);

    let safe_address: Address = proxy_wallet
        .parse()
        .map_err(|_| RedeemError::ProviderError("Invalid proxy wallet".to_string()))?;

    let positions = fetch_redeemable_positions(proxy_wallet).await?;
    if positions.is_empty() {
        return Ok(Vec::new());
    }

    info!("Found {} redeemable position(s)", positions.len());

    let mut results = Vec::with_capacity(positions.len());
    let mut seen = std::collections::HashSet::new();

    for position in positions {
        if !seen.insert(position.condition_id.clone()) {
            continue;
        }

        let mut result = RedemptionResult {
            condition_id: position.condition_id.clone(),
            title: position.title.clone(),
            outcome: position.outcome.clone(),
            size: position.size,
            neg_risk: position.negative_risk,
            tx_hash: None,
            error: None,
        };

        match redeem_via_safe(
            safe_address,
            &position.condition_id,
            position.negative_risk,
            &wallet,
            POLYGON_RPC_URL,
        ).await {
            Ok(tx_hash) => {
                info!("Redeemed: {} - TX: {:?}", position.title, tx_hash);
                result.tx_hash = Some(tx_hash);
            }
            Err(e) => {
                warn!("Failed: {} - {}", position.title, e);
                result.error = Some(e.to_string());
            }
        }

        results.push(result);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    Ok(results)
}

pub async fn redeem_single(condition_id: &str, neg_risk: bool) -> Result<TxHash> {
    let private_key = load_private_key()?;
    let proxy_wallet = load_proxy_wallet()?;

    let wallet: LocalWallet = private_key.trim_start_matches("0x")
        .parse()
        .map_err(|e: WalletError| RedeemError::ProviderError(e.to_string()))?;
    let wallet = wallet.with_chain_id(POLYGON_CHAIN_ID);

    let safe_address: Address = proxy_wallet
        .parse()
        .map_err(|_| RedeemError::ProviderError("Invalid proxy wallet".to_string()))?;

    redeem_via_safe(safe_address, condition_id, neg_risk, &wallet, POLYGON_RPC_URL).await
}

pub async fn redeem_all() -> Result<Vec<RedemptionResult>> {
    redeem_all_positions(&load_proxy_wallet()?).await
}

pub async fn create_signer_provider(
    rpc_url: &str,
    private_key: &str,
    chain_id: u64,
) -> Result<Arc<SignerMiddleware<Provider<Http>, LocalWallet>>> {
    let provider = Provider::<Http>::try_from(rpc_url)
        .map_err(|e| RedeemError::ProviderError(e.to_string()))?;
    let wallet: LocalWallet = private_key.trim_start_matches("0x")
        .parse()
        .map_err(|e: WalletError| RedeemError::ProviderError(e.to_string()))?;
    Ok(Arc::new(SignerMiddleware::new(provider, wallet.with_chain_id(chain_id))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_condition_id() {
        let valid = "0xabcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
        assert!(parse_condition_id(valid).is_ok());
        assert!(parse_condition_id(&valid[2..]).is_ok());
        assert!(parse_condition_id("invalid").is_err());
    }

    #[test]
    fn test_contract_addresses() {
        assert!(CTF_CONTRACT.parse::<Address>().is_ok());
        assert!(NEG_RISK_CTF_CONTRACT.parse::<Address>().is_ok());
        assert!(USDC_ADDRESS.parse::<Address>().is_ok());
    }
}
