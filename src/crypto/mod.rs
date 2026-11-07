//! Vault encryption primitives (ADR 0006).
//!
//! Key hierarchy: every protected vault has a random 256-bit master key.
//! Note files are AES-256-GCM ciphertext under that key; the database is
//! SQLCipher keyed with the same master key (raw hex form). The master key
//! itself never touches disk unencrypted — it is stored as an
//! AES-256-GCM-wrapped blob whose wrapping key comes from Argon2id over the
//! owner's passphrase, and optionally cached in the OS keychain as a
//! convenience wrapper (never a second credential).
//!
//! Friction budget (owner's binding constraint): passphrase at session
//! unlock, relock on sleep/manual only. Nothing in this module may add
//! interaction beyond that.

use std::path::Path;

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// Magic prefix identifying an encrypted note file (grill finding: format
/// must be self-describing so plaintext/ciphertext is detectable on read).
pub const FILE_MAGIC: &[u8] = b"CERE1";
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

/// Argon2id parameters — OWASP m=19MiB, t=2, p=1 territory. Stored per vault
/// so future tuning doesn't break old vaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KdfParams {
    pub salt_b64: String,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl KdfParams {
    pub fn generate() -> Self {
        let mut salt = [0u8; 16];
        getrandom::getrandom(&mut salt).expect("system RNG failure");
        Self {
            salt_b64: B64.encode(salt),
            m_cost: 19_452,
            t_cost: 2,
            p_cost: 1,
        }
    }

    pub fn salt(&self) -> Result<Vec<u8>> {
        B64.decode(&self.salt_b64)
            .context("invalid kdf salt in vault config")
    }
}

/// The `[protection]` section of a profile's vault.toml.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtectionConfig {
    pub enabled: bool,
    pub kdf: Option<KdfParams>,
    /// AES-256-GCM wrapped master key: base64(nonce || ciphertext), where
    /// the wrapping key = Argon2id(passphrase, salt, params).
    pub wrapped_key_b64: Option<String>,
    /// Optional macOS keychain convenience handle (service name). Presence
    /// does NOT change credentials — the passphrase still works alone.
    pub keychain_service: Option<String>,
}

impl Default for ProtectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            kdf: None,
            wrapped_key_b64: None,
            keychain_service: None,
        }
    }
}

/// An unlocked vault's in-memory key context. Held only while the session
/// is unlocked (CLI: per invocation; GUI/MCP: per session).
#[derive(Clone)]
pub struct VaultKey {
    key: [u8; KEY_LEN],
}

impl std::fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never leak key material through Debug.
        write!(f, "VaultKey(***redacted***)")
    }
}

impl VaultKey {
    pub fn generate() -> Self {
        let mut key = [0u8; KEY_LEN];
        getrandom::getrandom(&mut key).expect("system RNG failure");
        Self { key }
    }

    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self { key: bytes }
    }

    /// SQLCipher pragma value. Uses the STRING form (SQLCipher applies its
    /// internal KDF to it) because SQLite's PRAGMA grammar rejects bare
    /// blob literals like x'…'. Base64 alphabet contains no single quotes,
    /// so the quoting is safe.
    pub fn sqlcipher_pragma_value(&self) -> String {
        format!("'{}'", B64.encode(self.key))
    }

    fn aead(&self) -> Aes256Gcm {
        Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key))
    }

    /// Encrypt plaintext bytes; output = magic || version || nonce || ct.
    pub fn encrypt_bytes(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let aead = self.aead();
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = aead
            .encrypt(&nonce, plaintext)
            .map_err(|_| anyhow!("encryption failure"))?;
        let mut out = Vec::with_capacity(FILE_MAGIC.len() + 1 + NONCE_LEN + ct.len());
        out.extend_from_slice(FILE_MAGIC);
        out.push(1u8); // format version
        out.extend_from_slice(nonce.as_slice());
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// Decrypt bytes produced by [`encrypt_bytes`]. Detects plaintext
    /// passthrough? NO — callers must use [`looks_encrypted`] and handle
    /// plaintext explicitly; this function only accepts genuine ciphertext.
    pub fn decrypt_bytes(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        let header = FILE_MAGIC.len() + 1 + NONCE_LEN;
        if bytes.len() < header || &bytes[..FILE_MAGIC.len()] != FILE_MAGIC {
            bail!("not a Cerebro-encrypted file (missing magic)");
        }
        let version = bytes[FILE_MAGIC.len()];
        if version != 1 {
            bail!("unsupported encryption format version {version}");
        }
        let nonce = Nonce::from_slice(&bytes[FILE_MAGIC.len() + 1..FILE_MAGIC.len() + 1 + NONCE_LEN]);
        let aead = self.aead();
        aead.decrypt(nonce, &bytes[header..])
            .map_err(|_| anyhow!("decryption failure (wrong key or corrupted file)"))
    }

    /// Wrap the master key under an Argon2id-derived wrapping key.
    pub fn wrap_with_passphrase(&self, passphrase: &str, params: &KdfParams) -> Result<String> {
        let wrap_key = derive_key(passphrase, params)?;
        let aead = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&wrap_key));
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = aead
            .encrypt(&nonce, self.key.as_slice())
            .map_err(|_| anyhow!("encryption failure"))?;
        let mut blob = Vec::with_capacity(NONCE_LEN + ct.len());
        blob.extend_from_slice(nonce.as_slice());
        blob.extend_from_slice(&ct);
        Ok(B64.encode(blob))
    }

    /// Unwrap a passphrase-wrapped master key. Fails on wrong passphrase.
    pub fn unwrap_with_passphrase(wrapped_b64: &str, passphrase: &str, params: &KdfParams) -> Result<Self> {
        let wrap_key = derive_key(passphrase, params)?;
        let blob = B64.decode(wrapped_b64)
            .context("invalid wrapped key blob")?;
        if blob.len() < NONCE_LEN + KEY_LEN {
            bail!("wrapped key blob too short");
        }
        let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
        let aead = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&wrap_key));
        let pt = aead
            .decrypt(Nonce::from_slice(nonce_bytes), ct)
            .map_err(|_| anyhow!("wrong passphrase"))?;
        let arr: [u8; KEY_LEN] = pt
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("unwrapped key has wrong length"))?;
        Ok(Self::from_bytes(arr))
    }

    /// Unwrap the master key using a vault config's stored protection params.
    /// Convenience for callers holding a loaded ProfileConfig.
    pub fn unwrap_from_config(
        cfg: &crate::config::profile::ProfileConfig,
        passphrase: &str,
    ) -> Result<Self> {
        let kdf = cfg
            .protection
            .kdf
            .as_ref()
            .ok_or_else(|| anyhow!("vault config missing KDF parameters"))?;
        let wrapped = cfg
            .protection
            .wrapped_key_b64
            .as_deref()
            .ok_or_else(|| anyhow!("vault config missing wrapped master key"))?;
        Self::unwrap_with_passphrase(wrapped, passphrase, kdf)
    }

    /// Convenience cache (macOS Keychain via `keyring`): store the RAW master
    /// key so a session can unlock without the passphrase. This is a
    /// convenience wrapper ONLY — never a second credential or recovery path.
    pub fn cache_in_keychain(&self, service: &str) -> Result<()> {
        let entry = keyring::Entry::new("cerebro", service)?;
        entry.set_password(&B64.encode(self.key))?;
        Ok(())
    }

    /// Read the cached key back. Absent/empty keychain entry = None.
    pub fn from_keychain(service: &str) -> Result<Option<Self>> {
        let entry = keyring::Entry::new("cerebro", service)?;
        match entry.get_password() {
            Ok(b64) => {
                let bytes = B64.decode(b64).context("invalid keychain key blob")?;
                if bytes.len() != KEY_LEN {
                    bail!("keychain key has wrong length");
                }
                let arr: [u8; KEY_LEN] = bytes.as_slice().try_into().unwrap();
                Ok(Some(Self::from_bytes(arr)))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow!("keychain error: {e}")),
        }
    }

    /// Remove the convenience cache (used by vault decrypt / disable).
    pub fn remove_from_keychain(service: &str) -> Result<()> {
        let entry = keyring::Entry::new("cerebro", service)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow!("keychain error: {e}")),
        }
    }
}

/// True when the file bytes carry Cerebro's encryption magic.
pub fn looks_encrypted(bytes: &[u8]) -> bool {
    bytes.len() > FILE_MAGIC.len() && &bytes[..FILE_MAGIC.len()] == FILE_MAGIC
}

/// True when a note FILE on disk is ciphertext.
pub fn file_looks_encrypted(path: &Path) -> bool {
    match std::fs::read(path) {
        Ok(bytes) => looks_encrypted(&bytes),
        Err(_) => false,
    }
}

/// Derive a 32-byte wrapping key from a passphrase (Argon2id v0x13).
pub fn derive_key(passphrase: &str, params: &KdfParams) -> Result<[u8; KEY_LEN]> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let salt = params.salt()?;
    let p = Params::new(params.m_cost, params.t_cost, params.p_cost, Some(KEY_LEN))
        .map_err(|e| anyhow!("invalid Argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let mut out = [0u8; KEY_LEN];
    argon
        .hash_password_into(passphrase.as_bytes(), &salt, &mut out)
        .map_err(|e| anyhow!("argon2 failure: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_round_trip() {
        let key = VaultKey::generate();
        let pt = "# Note\n\nZażółć gęślą jaźń 🦀\n".as_bytes();
        let ct = key.encrypt_bytes(pt).unwrap();
        assert!(looks_encrypted(&ct));
        assert_ne!(ct, pt.to_vec());
        let back = key.decrypt_bytes(&ct).unwrap();
        assert_eq!(back, pt.to_vec());
    }

    #[test]
    fn wrong_key_fails() {
        let ct = VaultKey::generate().encrypt_bytes(b"secret").unwrap();
        let err = VaultKey::generate().decrypt_bytes(&ct).unwrap_err();
        assert!(err.to_string().contains("wrong key or corrupted"));
    }

    #[test]
    fn corrupted_ciphertext_fails() {
        let key = VaultKey::generate();
        let mut ct = key.encrypt_bytes(b"secret").unwrap();
        let last = ct.len() - 1;
        ct[last] ^= 0xFF;
        assert!(key.decrypt_bytes(&ct).is_err());
    }

    #[test]
    fn truncated_file_rejected() {
        let key = VaultKey::generate();
        let ct = key.encrypt_bytes(b"secret").unwrap();
        assert!(key.decrypt_bytes(&ct[..ct.len() - 3]).is_err());
        assert!(key.decrypt_bytes(&ct[..3]).is_err());
    }

    #[test]
    fn plaintext_bytes_are_not_magic() {
        assert!(!looks_encrypted(b"# plain markdown"));
        assert!(!looks_encrypted(b""));
    }

    #[test]
    fn wrap_unwrap_round_trip() {
        let key = VaultKey::generate();
        let params = KdfParams::generate();
        let wrapped = key.wrap_with_passphrase("correct horse battery staple", &params).unwrap();
        let unwrapped = VaultKey::unwrap_with_passphrase(&wrapped, "correct horse battery staple", &params).unwrap();
        // Same key material: ciphertext under one decrypts under the other.
        let ct = key.encrypt_bytes(b"check").unwrap();
        assert_eq!(unwrapped.decrypt_bytes(&ct).unwrap(), b"check");
    }

    #[test]
    fn wrong_passphrase_unwrap_fails() {
        let key = VaultKey::generate();
        let params = KdfParams::generate();
        let wrapped = key.wrap_with_passphrase("right", &params).unwrap();
        assert!(VaultKey::unwrap_with_passphrase(&wrapped, "wrong", &params).is_err());
    }

    #[test]
    fn sqlcipher_pragma_shape() {
        let key = VaultKey::from_bytes([0xAB; 32]);
        let v = key.sqlcipher_pragma_value();
        assert!(v.starts_with('\'') && v.ends_with('\''));
        // base64 of 32 bytes = 44 chars + 2 quotes
        assert_eq!(v.len(), 46);
        assert!(!v[1..v.len() - 1].contains('\''));
    }

    #[test]
    fn key_debug_is_redacted() {
        let key = VaultKey::generate();
        let dbg = format!("{key:?}");
        assert!(!dbg.contains(&format!("{:02x}", key.key[0])));
        assert!(dbg.contains("redacted"));
    }
}
