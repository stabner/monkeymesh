//! Ed25519 keys, BIP39 mnemonics, SLIP-0010 derivation, and encrypted vaults.

mod secret_file;
mod slip10;
mod vault;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use mesh_types::Address;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub use bip39::{Language, Mnemonic};
pub use secret_file::{
    mint_secret_hex, restrict_secret_file, write_secret_file, write_secret_file_no_clobber,
};
pub use slip10::{account_path, derive_ed25519_key, MESH_ACCOUNT_PATH, MESH_COIN_TYPE};
pub use vault::{
    VaultFile, VaultPayload, MIN_VAULT_PASSWORD_CHARS, VAULT_MAGIC_V1, VAULT_MAGIC_V2,
};

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("invalid public key")]
    InvalidPublicKey,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("invalid key length")]
    InvalidKeyLength,
    #[error("mnemonic error: {0}")]
    Mnemonic(String),
    #[error("derivation error: {0}")]
    Derivation(String),
    #[error("vault error: {0}")]
    Vault(String),
}

/// BIP39 strength used for new MonkeyMesh wallets (256-bit entropy → 24 words).
pub const DEFAULT_WORD_COUNT: usize = 24;

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Keypair {
    secret: [u8; 32],
}

impl Keypair {
    pub fn generate() -> Self {
        let signing = SigningKey::generate(&mut OsRng);
        Self {
            secret: signing.to_bytes(),
        }
    }

    pub fn from_bytes(secret: [u8; 32]) -> Self {
        Self { secret }
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.secret
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.secret)
    }

    pub fn from_hex(s: &str) -> Result<Self, CryptoError> {
        let bytes = hex::decode(s).map_err(|_| CryptoError::InvalidKeyLength)?;
        if bytes.len() != 32 {
            return Err(CryptoError::InvalidKeyLength);
        }
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&bytes);
        Ok(Self { secret })
    }

    /// Create a new 24-word BIP39 mnemonic (English).
    pub fn generate_mnemonic() -> Result<Mnemonic, CryptoError> {
        Mnemonic::generate_in(Language::English, DEFAULT_WORD_COUNT)
            .map_err(|e| CryptoError::Mnemonic(e.to_string()))
    }

    /// Parse a BIP39 phrase (12/15/18/21/24 words).
    pub fn mnemonic_from_phrase(phrase: &str) -> Result<Mnemonic, CryptoError> {
        Mnemonic::parse_in_normalized(Language::English, phrase.trim())
            .map_err(|e| CryptoError::Mnemonic(e.to_string()))
    }

    /// Derive spending key at hardened address index `N`
    /// (`m/44'/999778'/0'/0'/N'`).
    pub fn from_mnemonic_index(
        mnemonic: &Mnemonic,
        passphrase: &str,
        index: u32,
    ) -> Result<Self, CryptoError> {
        let seed = mnemonic.to_seed(passphrase);
        let path = account_path(index);
        let secret = derive_ed25519_key(&seed, &path)?;
        Ok(Self { secret })
    }

    /// Derive the primary spending key from a mnemonic via SLIP-0010 Ed25519.
    /// Path: [`MESH_ACCOUNT_PATH`] (`m/44'/999778'/0'/0'/0'`).
    pub fn from_mnemonic(mnemonic: &Mnemonic, passphrase: &str) -> Result<Self, CryptoError> {
        Self::from_mnemonic_index(mnemonic, passphrase, 0)
    }

    fn signing_key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.secret)
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key().verifying_key()
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.verifying_key().to_bytes()
    }

    pub fn address(&self) -> Address {
        Address::from_pubkey_bytes(&self.public_key_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key().sign(message).to_bytes()
    }
}

pub fn verify(pubkey: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> Result<(), CryptoError> {
    let vk = VerifyingKey::from_bytes(pubkey).map_err(|_| CryptoError::InvalidPublicKey)?;
    let sig = Signature::from_bytes(signature);
    vk.verify(message, &sig)
        .map_err(|_| CryptoError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_roundtrip() {
        let kp = Keypair::generate();
        let msg = b"monkeymesh";
        let sig = kp.sign(msg);
        verify(&kp.public_key_bytes(), msg, &sig).unwrap();
    }

    #[test]
    fn mnemonic_derives_stable_key() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let m = Keypair::mnemonic_from_phrase(phrase).unwrap();
        let a = Keypair::from_mnemonic(&m, "").unwrap();
        let b = Keypair::from_mnemonic(&m, "").unwrap();
        assert_eq!(a.to_bytes(), b.to_bytes());
        assert_ne!(a.address(), Address::default());
    }

    #[test]
    fn vault_roundtrip() {
        let m = Keypair::generate_mnemonic().unwrap();
        let phrase = m.to_string();
        let kp = Keypair::from_mnemonic(&m, "").unwrap();
        let vault = VaultFile::encrypt_mnemonic(
            &phrase,
            "correct horse battery",
            &kp.address().to_string(),
        )
        .unwrap();
        let unlocked = vault.unlock("correct horse battery").unwrap();
        assert_eq!(unlocked.mnemonic.as_deref(), Some(phrase.as_str()));
        let kp2 = Keypair::from_mnemonic(
            &Keypair::mnemonic_from_phrase(unlocked.mnemonic.as_ref().unwrap()).unwrap(),
            "",
        )
        .unwrap();
        assert_eq!(kp.to_bytes(), kp2.to_bytes());
        assert!(vault.unlock("wrong").is_err());
    }

    #[test]
    fn vault_address_readable_without_unlock() {
        let m = Keypair::generate_mnemonic().unwrap();
        let phrase = m.to_string();
        let kp = Keypair::from_mnemonic(&m, "").unwrap();
        let addr = kp.address().to_string();
        let vault =
            VaultFile::encrypt_mnemonic(&phrase, "correct horse battery", &addr).unwrap();
        let json = vault.to_json_pretty().unwrap();
        let loaded = VaultFile::from_json(&json).unwrap();
        assert_eq!(loaded.address, addr);
        assert_eq!(loaded.magic, VAULT_MAGIC_V2);
        // Wrong password still fails unlock — address was never secret.
        assert!(loaded.unlock("wrong").is_err());
    }

    #[test]
    fn vault_rejects_short_password() {
        let m = Keypair::generate_mnemonic().unwrap();
        let phrase = m.to_string();
        let kp = Keypair::from_mnemonic(&m, "").unwrap();
        let err = VaultFile::encrypt_mnemonic(&phrase, "short-pw", &kp.address().to_string())
            .unwrap_err();
        assert!(err.to_string().contains("15"));
    }
}
