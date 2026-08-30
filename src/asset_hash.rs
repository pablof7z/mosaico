//! Exact-byte SHA-256 identity for attachment descriptors.

use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Sha256Hash([u8; 32]);

impl Sha256Hash {
    pub(crate) fn of(bytes: impl AsRef<[u8]>) -> Self {
        let mut hash = [0; 32];
        hash.copy_from_slice(&Sha256::digest(bytes));
        Self(hash)
    }

    pub(crate) fn from_hex(value: &str) -> Result<Self, String> {
        if value.len() != 64 {
            return Err("must contain exactly 64 hexadecimal characters".into());
        }

        let mut hash = [0; 32];
        for (index, byte) in hash.iter_mut().enumerate() {
            let offset = index * 2;
            *byte = (hex_digit(value.as_bytes()[offset])? << 4)
                | hex_digit(value.as_bytes()[offset + 1])?;
        }
        Ok(Self(hash))
    }

    pub(crate) fn to_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut value = String::with_capacity(64);
        for byte in self.0 {
            value.push(HEX[(byte >> 4) as usize] as char);
            value.push(HEX[(byte & 0x0f) as usize] as char);
        }
        value
    }
}

fn hex_digit(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("contains a non-hexadecimal character".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::Sha256Hash;

    #[test]
    fn round_trips_the_sha256_of_exact_bytes() {
        let hash = Sha256Hash::of(b"attachment");
        assert_eq!(
            hash.to_hex(),
            "602a5e69c3021bdbd3d25156a02d2cbb467605b8203248eea6af3fb42168d663"
        );
        assert_eq!(Sha256Hash::from_hex(&hash.to_hex()).unwrap(), hash);
    }

    #[test]
    fn rejects_noncanonical_hex() {
        assert!(Sha256Hash::from_hex("not-a-hash").is_err());
        assert!(Sha256Hash::from_hex(&"g".repeat(64)).is_err());
    }
}
