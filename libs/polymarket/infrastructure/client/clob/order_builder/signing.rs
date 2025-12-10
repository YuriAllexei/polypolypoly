//! EIP-712 signing logic for CTF Exchange orders

use super::encoding::{encode_address, encode_uint256, encode_uint8};
use super::types::Order;
use super::super::constants::*;
use ethers::types::U256;
use ethers::utils::keccak256;

/// Compute the full EIP-712 message hash
///
/// hash = keccak256("\x19\x01" || domainSeparator || structHash)
pub fn compute_eip712_hash(order: &Order, chain_id: u64, neg_risk: bool) -> [u8; 32] {
    let domain_separator = compute_domain_separator(chain_id, neg_risk);
    let struct_hash = compute_struct_hash(order);

    let mut message = Vec::with_capacity(66);
    message.extend_from_slice(b"\x19\x01");
    message.extend_from_slice(&domain_separator);
    message.extend_from_slice(&struct_hash);

    keccak256(&message)
}

/// Compute the EIP-712 domain separator
///
/// domainSeparator = keccak256(
///     keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)") ||
///     keccak256(name) ||
///     keccak256(version) ||
///     chainId ||
///     verifyingContract
/// )
pub fn compute_domain_separator(chain_id: u64, neg_risk: bool) -> [u8; 32] {
    let type_hash = keccak256(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );

    let name_hash = keccak256(EIP712_DOMAIN_NAME.as_bytes());
    let version_hash = keccak256(EIP712_DOMAIN_VERSION.as_bytes());

    let mut encoded = Vec::new();
    encoded.extend_from_slice(&type_hash);
    encoded.extend_from_slice(&name_hash);
    encoded.extend_from_slice(&version_hash);
    encoded.extend_from_slice(&encode_uint256(U256::from(chain_id)));
    encoded.extend_from_slice(&encode_address(get_exchange_address(neg_risk)));

    keccak256(&encoded)
}

/// Compute the struct hash for an Order
///
/// structHash = keccak256(
///     keccak256("Order(...)") ||
///     salt || maker || signer || taker || tokenId ||
///     makerAmount || takerAmount || expiration ||
///     nonce || feeRateBps || side || signatureType
/// )
pub fn compute_struct_hash(order: &Order) -> [u8; 32] {
    let type_hash = keccak256(
        b"Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)",
    );

    let mut encoded = Vec::new();
    encoded.extend_from_slice(&type_hash);
    encoded.extend_from_slice(&encode_uint256(order.salt));
    encoded.extend_from_slice(&encode_address(order.maker));
    encoded.extend_from_slice(&encode_address(order.signer));
    encoded.extend_from_slice(&encode_address(order.taker));
    encoded.extend_from_slice(&encode_uint256(order.token_id));
    encoded.extend_from_slice(&encode_uint256(order.maker_amount));
    encoded.extend_from_slice(&encode_uint256(order.taker_amount));
    encoded.extend_from_slice(&encode_uint256(order.expiration));
    encoded.extend_from_slice(&encode_uint256(order.nonce));
    encoded.extend_from_slice(&encode_uint256(order.fee_rate_bps));
    encoded.extend_from_slice(&encode_uint8(order.side));
    encoded.extend_from_slice(&encode_uint8(order.signature_type));

    keccak256(&encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::types::Address;

    #[test]
    fn test_domain_separator_computation() {
        let domain_sep = compute_domain_separator(POLYGON_CHAIN_ID, false);
        assert_eq!(domain_sep.len(), 32);
        // Domain separator should be deterministic
        let domain_sep2 = compute_domain_separator(POLYGON_CHAIN_ID, false);
        assert_eq!(domain_sep, domain_sep2);
    }

    #[test]
    fn test_domain_separator_matches_python() {
        // Expected domain separator from Python (non-neg_risk)
        let expected_regular = hex::decode("1a573e3617c78403b5b4b892827992f027b03d4eaf570048b8ee8cdd84d151be").unwrap();

        // Expected domain separator from Python (neg_risk)
        let expected_neg_risk = hex::decode("82cb6aa85babb812f4b521a12b10f0cbc68d2b44be7bc02c047004f544adb49f").unwrap();

        let domain_sep_regular = compute_domain_separator(POLYGON_CHAIN_ID, false);
        let domain_sep_neg_risk = compute_domain_separator(POLYGON_CHAIN_ID, true);

        assert_eq!(
            domain_sep_regular.to_vec(),
            expected_regular,
            "Regular domain separator mismatch. Got: {:?}",
            hex::encode(domain_sep_regular)
        );
        assert_eq!(
            domain_sep_neg_risk.to_vec(),
            expected_neg_risk,
            "Neg risk domain separator mismatch. Got: {:?}",
            hex::encode(domain_sep_neg_risk)
        );
    }

    #[test]
    fn test_domain_separator_neg_risk_differs() {
        let domain_sep_regular = compute_domain_separator(POLYGON_CHAIN_ID, false);
        let domain_sep_neg_risk = compute_domain_separator(POLYGON_CHAIN_ID, true);

        // Domain separators should differ because different exchange addresses
        assert_ne!(domain_sep_regular, domain_sep_neg_risk);
    }

    #[test]
    fn test_struct_hash_matches_python() {
        // Test order with known values from Python:
        // Struct hash: 0x0ed29e68e2dde42b23125c3b6cdf6080daa8a01494743da240566e02439cc370
        // EIP-712 message hash: 0x36ea8c22435f8c4a2804e77be5074f23f98101af0a339564693cd0b186ebda46

        let maker: Address = "0x497284Cd581433f3C8224F07556a8d903113E0D3"
            .parse()
            .unwrap();

        let order = Order {
            salt: U256::from(12345u64),
            maker,
            signer: maker,
            taker: zero_address(),
            token_id: U256::from_dec_str("87681536460342357667165150330318852851476971055929009934844581402585803923513").unwrap(),
            maker_amount: U256::from(16400000u64),
            taker_amount: U256::from(40000000u64),
            expiration: U256::zero(),
            nonce: U256::zero(),
            fee_rate_bps: U256::zero(),
            side: SIDE_BUY,
            signature_type: SIGNATURE_TYPE_EOA,
        };

        let struct_hash = compute_struct_hash(&order);
        let expected_struct_hash = hex::decode("0ed29e68e2dde42b23125c3b6cdf6080daa8a01494743da240566e02439cc370").unwrap();

        assert_eq!(
            struct_hash.to_vec(),
            expected_struct_hash,
            "Struct hash mismatch. Got: 0x{}",
            hex::encode(struct_hash)
        );

        let eip712_hash = compute_eip712_hash(&order, POLYGON_CHAIN_ID, false);
        let expected_eip712_hash = hex::decode("36ea8c22435f8c4a2804e77be5074f23f98101af0a339564693cd0b186ebda46").unwrap();

        assert_eq!(
            eip712_hash.to_vec(),
            expected_eip712_hash,
            "EIP-712 hash mismatch. Got: 0x{}",
            hex::encode(eip712_hash)
        );
    }
}
