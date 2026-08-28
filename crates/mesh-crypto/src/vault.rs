//! Password-encrypted wallet vault.
//!
//! **v2 (new writes):** RFC 9106 Argon2id second recommended option
//! (64 MiB, t=3, p=4) + XChaCha20-Poly1305 (192-bit random nonce).
//! Password floor is NIST SP 800-63B-4 single-factor: 15 characters.
//!
//! **v1 (still unlocks):** Argon2id 19 MiB / t=2 / p=1 + ChaCha20-Poly1305.
//! Successful unlock of a v1 vault can be re-encrypted to v2 by the caller.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, XChaCha20Poly1305, XNonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::CryptoError;

pub const VAULT_MAGIC_V1: &str = "mesh-vault-v1";
pub const VAULT_MAGIC_V2: &str = "mesh-vault-v2";

/// NIST SP 800-63B-4: memorized secret used as the only factor.
pub const MIN_VAULT_PASSWORD_CHARS: usize = 15;

/// RFC 9106 §4 second recommended option (memory-constrained but strong).
const V2_M_KIB: u32 = 65_536;
const V2_T: u32 = 3;
const V2_P: u32 = 4;

/// OWASP / previous mesh-vault-v1 interactive baseline.
const V1_M_KIB: u32 = 19_456;
const V1_T: u32 = 2;
const V1_P: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultFile {
    pub magic: String,
    pub kdf: String,
    pub cipher: String,
    pub salt_b64: String,
    pub nonce_b64: String,
    pub ciphertext_b64: String,
    /// Convenience: address at encryption time (not secret).
    pub address: String,
    pub path: String,
    /// Argon2 memory in KiB (v2+). Absent on v1 files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m_kib: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Zeroize)]
#[zeroize(drop)]
pub struct VaultPayload {
    /// BIP39 mnemonic phrase (preferred).
    pub mnemonic: Option<String>,
    /// Legacy raw secret hex (migration only).
    pub legacy_secret_hex: Option<String>,
    pub path: String,
}

impl VaultFile {
    pub fn is_v1(&self) -> bool {
        self.magic == VAULT_MAGIC_V1
    }

    pub fn encrypt_mnemonic(
        mnemonic: &str,
        password: &str,
        address: &str,
    ) -> Result<Self, CryptoError> {
        let payload = VaultPayload {
            mnemonic: Some(mnemonic.to_string()),
            legacy_secret_hex: None,
            path: crate::MESH_ACCOUNT_PATH.to_string(),
        };
        Self::encrypt_payload(&payload, password, address)
    }

    pub fn encrypt_legacy_key(
        secret_hex: &str,
        password: &str,
        address: &str,
    ) -> Result<Self, CryptoError> {
        let payload = VaultPayload {
            mnemonic: None,
            legacy_secret_hex: Some(secret_hex.to_string()),
            path: "legacy".into(),
        };
        Self::encrypt_payload(&payload, password, address)
    }

    fn encrypt_payload(
        payload: &VaultPayload,
        password: &str,
        address: &str,
    ) -> Result<Self, CryptoError> {
        if password.chars().count() < MIN_VAULT_PASSWORD_CHARS {
            return Err(CryptoError::Vault(format!(
                "password must be at least {MIN_VAULT_PASSWORD_CHARS} characters (NIST SP 800-63B-4)"
            )));
        }
        let mut salt = [0u8; 16];
        let mut nonce_bytes = [0u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

        let mut key = derive_key(password.as_bytes(), &salt, V2_M_KIB, V2_T, V2_P)?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
        key.zeroize();
        let nonce = XNonce::from_slice(&nonce_bytes);
        let plaintext = serde_json::to_vec(payload).map_err(|e| CryptoError::Vault(e.to_string()))?;
        let aad = aad_v2(address, &payload.path);
        let ciphertext = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext.as_ref(),
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| CryptoError::Vault("encryption failed".into()))?;

        Ok(Self {
            magic: VAULT_MAGIC_V2.into(),
            kdf: "argon2id".into(),
            cipher: "xchacha20poly1305".into(),
            salt_b64: b64(&salt),
            nonce_b64: b64(&nonce_bytes),
            ciphertext_b64: b64(&ciphertext),
            address: address.to_string(),
            path: payload.path.clone(),
            m_kib: Some(V2_M_KIB),
            t: Some(V2_T),
            p: Some(V2_P),
        })
    }

    pub fn unlock(&self, password: &str) -> Result<VaultPayload, CryptoError> {
        match self.magic.as_str() {
            VAULT_MAGIC_V2 => self.unlock_v2(password),
            VAULT_MAGIC_V1 => self.unlock_v1(password),
            _ => Err(CryptoError::Vault("unsupported vault format".into())),
        }
    }

    /// Re-encrypt a successfully unlocked payload as v2 (call after v1 unlock).
    pub fn upgrade_to_v2(payload: &VaultPayload, password: &str, address: &str) -> Result<Self, CryptoError> {
        Self::encrypt_payload(payload, password, address)
    }

    fn unlock_v2(&self, password: &str) -> Result<VaultPayload, CryptoError> {
        let salt = from_b64(&self.salt_b64)?;
        let nonce_bytes = from_b64(&self.nonce_b64)?;
        let ciphertext = from_b64(&self.ciphertext_b64)?;
        if salt.len() != 16 || nonce_bytes.len() != 24 {
            return Err(CryptoError::Vault("bad vault parameters".into()));
        }
        let m = self.m_kib.unwrap_or(V2_M_KIB);
        let t = self.t.unwrap_or(V2_T);
        let p = self.p.unwrap_or(V2_P);
        let mut key = derive_key(password.as_bytes(), &salt, m, t, p)?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
        key.zeroize();
        let nonce = XNonce::from_slice(&nonce_bytes);
        let aad = aad_v2(&self.address, &self.path);
        let plaintext = cipher
            .decrypt(
                nonce,
                Payload {
                    msg: ciphertext.as_ref(),
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| CryptoError::Vault("wrong password or corrupt vault".into()))?;
        serde_json::from_slice(&plaintext).map_err(|e| CryptoError::Vault(e.to_string()))
    }

    fn unlock_v1(&self, password: &str) -> Result<VaultPayload, CryptoError> {
        let salt = from_b64(&self.salt_b64)?;
        let nonce_bytes = from_b64(&self.nonce_b64)?;
        let ciphertext = from_b64(&self.ciphertext_b64)?;
        if salt.len() != 16 || nonce_bytes.len() != 12 {
            return Err(CryptoError::Vault("bad vault parameters".into()));
        }
        let mut key = derive_key(password.as_bytes(), &salt, V1_M_KIB, V1_T, V1_P)?;
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
        key.zeroize();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|_| CryptoError::Vault("wrong password or corrupt vault".into()))?;
        serde_json::from_slice(&plaintext).map_err(|e| CryptoError::Vault(e.to_string()))
    }

    pub fn to_json_pretty(&self) -> Result<String, CryptoError> {
        serde_json::to_string_pretty(self).map_err(|e| CryptoError::Vault(e.to_string()))
    }

    pub fn from_json(s: &str) -> Result<Self, CryptoError> {
        serde_json::from_str(s.trim()).map_err(|e| CryptoError::Vault(e.to_string()))
    }
}

fn aad_v2(address: &str, path: &str) -> String {
    format!("mesh-vault-v2|{address}|{path}")
}

fn derive_key(
    password: &[u8],
    salt: &[u8],
    m_kib: u32,
    t: u32,
    p: u32,
) -> Result<[u8; 32], CryptoError> {
    let params = Params::new(m_kib, t, p, Some(32)).map_err(|e| CryptoError::Vault(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(password, salt, &mut out)
        .map_err(|e| CryptoError::Vault(e.to_string()))?;
    Ok(out)
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn from_b64(s: &str) -> Result<Vec<u8>, CryptoError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| CryptoError::Vault(e.to_string()))
}
