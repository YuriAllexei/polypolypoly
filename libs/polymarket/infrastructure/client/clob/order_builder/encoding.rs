//! ABI encoding helpers for EIP-712 hashing

use ethers::types::{Address, U256};

/// Encode a U256 as 32 bytes (big-endian, left-padded)
pub fn encode_uint256(value: U256) -> [u8; 32] {
    let mut buf = [0u8; 32];
    value.to_big_endian(&mut buf);
    buf
}

/// Encode an address as 32 bytes (left-padded with zeros)
pub fn encode_address(addr: Address) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[12..].copy_from_slice(addr.as_bytes());
    buf
}

/// Encode a u8 as 32 bytes (left-padded with zeros)
pub fn encode_uint8(value: u8) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[31] = value;
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_uint256() {
        let value = U256::from(123456789u64);
        let encoded = encode_uint256(value);
        assert_eq!(encoded.len(), 32);

        // Verify big-endian encoding
        let decoded = U256::from_big_endian(&encoded);
        assert_eq!(decoded, value);
    }

    #[test]
    fn test_encode_address() {
        let addr: Address = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E"
            .parse()
            .unwrap();
        let encoded = encode_address(addr);
        assert_eq!(encoded.len(), 32);

        // First 12 bytes should be zeros
        assert_eq!(&encoded[..12], &[0u8; 12]);
    }

    #[test]
    fn test_encode_uint8() {
        let encoded = encode_uint8(1);
        assert_eq!(encoded.len(), 32);
        assert_eq!(encoded[31], 1);
        assert_eq!(&encoded[..31], &[0u8; 31]);
    }
}
