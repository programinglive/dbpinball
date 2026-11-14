//! Masking keys.
//!
//! A masking key is 32 bytes from the operating system CSPRNG. It never leaves
//! this module except as an opaque `MaskKey` value, and it is never serialized
//! into a plan, a log, an audit record, or a CLI argument.
//!
//! Production keys live in the OS keyring under `dbpinball-safe-sync`. Tests use
//! `MaskKey::from_bytes` directly and never touch the keyring.

use sha2::{Digest, Sha256};

use crate::domain::error::{CoreError, ErrorCode};

const KEYRING_SERVICE: &str = "dbpinball-safe-sync";

/// A 32-byte key used for keyed masking and filter digests.
#[derive(Clone)]
pub struct MaskKey {
    id: String,
    bytes: [u8; 32],
}

impl std::fmt::Debug for MaskKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Debug must never print the key material.
        formatter.write_str(&format!(
            "MaskKey {{ id: {:?}, bytes: <redacted> }}",
            self.id
        ))
    }
}

impl MaskKey {
    /// Build a key from explicit bytes. Used by tests and by the keyring loader.
    pub fn from_bytes(id: impl Into<String>, bytes: [u8; 32]) -> Self {
        Self {
            id: id.into(),
            bytes,
        }
    }

    /// Generate a key from the operating system CSPRNG.
    pub fn generate() -> Result<Self, CoreError> {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("masking key generation failed: {error}"),
            )
        })?;
        let id = key_id_for(&bytes);
        Ok(Self { id, bytes })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Keyed digest with a caller-supplied domain string.
    ///
    /// The domain is hashed together with the material, so the same input in two
    /// different fields produces two different outputs.
    pub fn hmac(&self, domain: &str, material: &[u8]) -> [u8; 32] {
        crate::safesync::mask::hmac_sha256(&self.bytes, domain, material)
    }

    /// The raw bytes, for the keyring encoder only.
    pub(crate) fn bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

/// `k-` plus the first 8 hex characters of the key's own digest.
///
/// The id is a label, not a secret: it tells the operator which key a plan used
/// without revealing the key.
pub fn key_id_for(bytes: &[u8; 32]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(8);
    for byte in digest.iter().take(4) {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("k-{hex}")
}

/// Where masking keys are stored.
pub trait MaskKeyStore {
    /// The current key, or `mask_key_missing` when none is configured.
    fn load(&self) -> Result<MaskKey, CoreError>;
    /// Persist a key. Never returns the key material.
    fn store(&self, key: &MaskKey) -> Result<(), CoreError>;
}

/// An in-memory store. Tests and the `test-keys` feature use this.
#[derive(Default, Clone)]
pub struct InMemoryMaskKeyStore {
    key: Option<MaskKey>,
}

impl InMemoryMaskKeyStore {
    pub fn with_key(key: MaskKey) -> Self {
        Self { key: Some(key) }
    }
}

impl MaskKeyStore for InMemoryMaskKeyStore {
    fn load(&self) -> Result<MaskKey, CoreError> {
        self.key.clone().ok_or_else(missing_key)
    }

    fn store(&self, key: &MaskKey) -> Result<(), CoreError> {
        // `Self` is immutable in this trait, so the caller replaces the store.
        // The in-memory store exists for tests, which construct it with a key.
        let _ = key;
        Ok(())
    }
}

/// The OS keyring store.
pub struct KeyringMaskKeyStore;

impl KeyringMaskKeyStore {
    fn entry(id: &str) -> Result<keyring::Entry, CoreError> {
        keyring::Entry::new(KEYRING_SERVICE, &format!("mask-key:{id}")).map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("masking key could not be opened: {error}"),
            )
        })
    }
}

impl MaskKeyStore for KeyringMaskKeyStore {
    fn load(&self) -> Result<MaskKey, CoreError> {
        // The current key id is stored under a fixed account, so a rotation
        // changes which key id is loaded without touching the spec.
        let pointer = keyring::Entry::new(KEYRING_SERVICE, "current").map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("masking key could not be opened: {error}"),
            )
        })?;

        let id = pointer.get_password().map_err(|_| missing_key())?;
        let entry = Self::entry(&id)?;
        let encoded = entry.get_password().map_err(|_| missing_key())?;
        let bytes = decode_hex(&encoded).ok_or_else(|| {
            CoreError::new(ErrorCode::Operation, "the stored mask key is malformed")
        })?;
        Ok(MaskKey::from_bytes(id, bytes))
    }

    fn store(&self, key: &MaskKey) -> Result<(), CoreError> {
        let entry = Self::entry(key.id())?;
        entry
            .set_password(&encode_hex(key.bytes()))
            .map_err(|error| {
                CoreError::new(
                    ErrorCode::Operation,
                    format!("masking key could not be stored: {error}"),
                )
            })?;

        let pointer = keyring::Entry::new(KEYRING_SERVICE, "current").map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("masking key could not be stored: {error}"),
            )
        })?;
        pointer.set_password(key.id()).map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("masking key could not be stored: {error}"),
            )
        })
    }
}

fn missing_key() -> CoreError {
    CoreError::with_detail(
        ErrorCode::Operation,
        "no masking key is configured; run `dbpinball safe-sync key rotate`",
        "mask_key_missing",
    )
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn decode_hex(encoded: &str) -> Option<[u8; 32]> {
    let encoded = encoded.trim();
    if encoded.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, chunk) in encoded.as_bytes().chunks(2).enumerate() {
        let text = std::str::from_utf8(chunk).ok()?;
        bytes[index] = u8::from_str_radix(text, 16).ok()?;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> MaskKey {
        MaskKey::from_bytes("k-test", [7u8; 32])
    }

    #[test]
    fn the_key_id_is_derived_and_stable() {
        let bytes = [7u8; 32];
        assert_eq!(key_id_for(&bytes), key_id_for(&bytes));
        assert!(key_id_for(&bytes).starts_with("k-"));
    }

    #[test]
    fn a_different_key_has_a_different_id() {
        assert_ne!(key_id_for(&[1u8; 32]), key_id_for(&[2u8; 32]));
    }

    #[test]
    fn generated_keys_are_32_bytes_and_distinct() {
        let first = MaskKey::generate().unwrap();
        let second = MaskKey::generate().unwrap();
        assert_ne!(first.id(), second.id());
        assert_eq!(first.bytes().len(), 32);
    }

    #[test]
    fn debug_never_prints_the_key_material() {
        let rendered = format!("{:?}", key());
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("07"));
    }

    #[test]
    fn a_missing_key_is_a_typed_refusal() {
        let store = InMemoryMaskKeyStore::default();
        let error = store.load().unwrap_err();
        assert_eq!(error.code, ErrorCode::Operation);
        assert_eq!(error.detail.as_deref(), Some("mask_key_missing"));
    }

    #[test]
    fn hex_round_trips() {
        let bytes = [9u8; 32];
        assert_eq!(decode_hex(&encode_hex(&bytes)), Some(bytes));
    }

    #[test]
    fn a_malformed_hex_string_is_rejected() {
        assert!(decode_hex("abc").is_none());
        assert!(decode_hex(&"zz".repeat(32)).is_none());
    }
}
