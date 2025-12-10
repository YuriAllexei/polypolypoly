//! Polymarket CTF Exchange Constants
//!
//! Contract addresses and EIP-712 domain constants for Polygon Mainnet.

use ethers::types::Address;

// ============================================================================
// Network Constants
// ============================================================================

/// Chain ID for Polygon Mainnet
pub const POLYGON_CHAIN_ID: u64 = 137;

// ============================================================================
// Contract Addresses (Polygon Mainnet)
// ============================================================================

/// CTF Exchange contract address (regular markets)
pub const CTF_EXCHANGE: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";

/// CTF Exchange contract address (neg_risk markets)
pub const NEG_RISK_CTF_EXCHANGE: &str = "0xC5d563A36AE78145C45a50134d48A1215220f80a";

/// Conditional Tokens (ERC1155) contract
pub const CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";

/// USDC contract on Polygon
pub const USDC_ADDRESS: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";

/// Zero address (for public orders)
pub const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";

// ============================================================================
// EIP-712 Domain Constants
// ============================================================================

/// EIP-712 domain name for CTF Exchange
pub const EIP712_DOMAIN_NAME: &str = "Polymarket CTF Exchange";

/// EIP-712 domain version
pub const EIP712_DOMAIN_VERSION: &str = "1";

// ============================================================================
// Signature Types
// ============================================================================

/// EOA signature type (direct wallet signing)
pub const SIGNATURE_TYPE_EOA: u8 = 0;

/// POLY_PROXY signature type (proxy wallet)
pub const SIGNATURE_TYPE_POLY_PROXY: u8 = 1;

/// POLY_GNOSIS_SAFE signature type
pub const SIGNATURE_TYPE_POLY_GNOSIS_SAFE: u8 = 2;

// ============================================================================
// Order Side Encoding
// ============================================================================

/// Buy side (0)
pub const SIDE_BUY: u8 = 0;

/// Sell side (1)
pub const SIDE_SELL: u8 = 1;

// ============================================================================
// Token Decimals
// ============================================================================

/// USDC has 6 decimal places
pub const TOKEN_DECIMALS: u32 = 6;

/// Multiplier for converting to token decimals (10^6)
pub const DECIMAL_MULTIPLIER: u64 = 1_000_000;

// ============================================================================
// Helper Functions
// ============================================================================

/// Parse the CTF Exchange address (regular markets)
pub fn exchange_address() -> Address {
    CTF_EXCHANGE.parse().expect("Invalid exchange address constant")
}

/// Parse the CTF Exchange address (neg_risk markets)
pub fn neg_risk_exchange_address() -> Address {
    NEG_RISK_CTF_EXCHANGE.parse().expect("Invalid neg_risk exchange address constant")
}

/// Get exchange address based on neg_risk flag
pub fn get_exchange_address(neg_risk: bool) -> Address {
    if neg_risk {
        neg_risk_exchange_address()
    } else {
        exchange_address()
    }
}

/// Parse the zero address
pub fn zero_address() -> Address {
    ZERO_ADDRESS.parse().expect("Invalid zero address constant")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_parsing() {
        // Ensure all address constants are valid
        let _ = exchange_address();
        let _ = neg_risk_exchange_address();
        let _ = zero_address();
    }

    #[test]
    fn test_chain_id() {
        assert_eq!(POLYGON_CHAIN_ID, 137);
    }

    #[test]
    fn test_decimal_multiplier() {
        assert_eq!(DECIMAL_MULTIPLIER, 1_000_000);
    }
}
