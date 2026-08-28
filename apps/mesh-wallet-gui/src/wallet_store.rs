//! Wallet persistence: encrypted BIP39 vault (+ legacy hex migration).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use mesh_crypto::{Keypair, VaultFile};
use serde::Deserialize;

#[derive(Deserialize)]
struct WalletConfig {
    rpc: Option<String>,
    wallet_key: Option<String>,
    /// Preferred vault path (encrypted seed). Defaults beside wallet_key as `.vault.json`.
    wallet_vault: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WalletKind {
    /// Password-locked BIP39 vault.
    Vault,
    /// Legacy plaintext hex key (insecure — migrate ASAP).
    LegacyHex,
}

pub struct LoadedWallet {
    pub key: Keypair,
    pub kind: WalletKind,
    pub vault_path: PathBuf,
    pub legacy_key_path: PathBuf,
    /// Set when a new mnemonic was just created (must show backup UI).
    pub fresh_mnemonic: Option<String>,
    /// Session mnemonic for HD address derivation (vault wallets only).
    pub mnemonic: Option<String>,
}

/// Comma-separated RPC list from env / config (first live URL is chosen later).
pub fn resolve_rpc_candidates() -> Vec<String> {
    let mut out = Vec::new();
    let push_list = |out: &mut Vec<String>, raw: &str| {
        for u in mesh_types::parse_rpc_list(raw) {
            if !out.iter().any(|x| x == &u) {
                out.push(u);
            }
        }
    };
    if let Ok(v) = std::env::var("MESH_RPC") {
        push_list(&mut out, &v);
    }
    if let Some(cfg) = load_config() {
        if let Some(rpc) = cfg.rpc {
            push_list(&mut out, &rpc);
        }
    }
    if out.is_empty() {
        out.push(mesh_types::default_seed_rpc_url());
        out.push(mesh_types::default_edge_rpc_url());
    }
    out
}

pub fn resolve_rpc_url() -> String {
    resolve_rpc_candidates()
        .into_iter()
        .next()
        .unwrap_or_else(mesh_types::default_seed_rpc_url)
}

pub fn resolve_legacy_key_path() -> PathBuf {
    if let Ok(v) = std::env::var("MESH_WALLET_KEY") {
        let p = PathBuf::from(v.trim());
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    if let Some(cfg) = load_config() {
        if let Some(k) = cfg.wallet_key {
            let p = PathBuf::from(k.trim());
            if p.is_absolute() {
                return p;
            }
            if let Some(base) = config_dir() {
                return base.join(p);
            }
            return p;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin) = exe.parent() {
            let beside = bin.join("..").join("data").join("wallet.key");
            if let Ok(canon) = fs::canonicalize(&beside) {
                return canon;
            }
            return beside;
        }
    }
    PathBuf::from("data/wallet.key")
}

pub fn resolve_vault_path() -> PathBuf {
    if let Ok(v) = std::env::var("MESH_WALLET_VAULT") {
        let p = PathBuf::from(v.trim());
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    if let Some(cfg) = load_config() {
        if let Some(k) = cfg.wallet_vault {
            let p = PathBuf::from(k.trim());
            if p.is_absolute() {
                return p;
            }
            if let Some(base) = config_dir() {
                return base.join(p);
            }
            return p;
        }
    }
    // Prefer Launchers/Wallet/data/wallet.vault.json
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin) = exe.parent() {
            let beside = bin.join("..").join("data").join("wallet.vault.json");
            return beside;
        }
    }
    let legacy = resolve_legacy_key_path();
    legacy.with_extension("vault.json")
}

pub fn vault_exists() -> bool {
    resolve_vault_path().exists()
}

pub fn legacy_exists() -> bool {
    resolve_legacy_key_path().exists()
}

pub fn unlock_vault(password: &str) -> Result<LoadedWallet> {
    let vault_path = resolve_vault_path();
    let raw = fs::read_to_string(&vault_path)
        .with_context(|| format!("read vault {}", vault_path.display()))?;
    let vault = VaultFile::from_json(&raw)?;
    let payload = vault.unlock(password)?;
    if vault.is_v1() && password.chars().count() >= mesh_crypto::MIN_VAULT_PASSWORD_CHARS {
        if let Ok(upgraded) = VaultFile::upgrade_to_v2(&payload, password, &vault.address) {
            if let Ok(json) = upgraded.to_json_pretty() {
                let _ = mesh_crypto::write_secret_file(&vault_path, json.as_bytes());
            }
        }
    }
    let key = if let Some(phrase) = payload.mnemonic.as_ref() {
        let m = Keypair::mnemonic_from_phrase(phrase)?;
        Keypair::from_mnemonic(&m, "")?
    } else if let Some(hex) = payload.legacy_secret_hex.as_ref() {
        Keypair::from_hex(hex.trim())?
    } else {
        bail!("vault has no mnemonic or legacy key");
    };
    Ok(LoadedWallet {
        key,
        kind: WalletKind::Vault,
        vault_path,
        legacy_key_path: resolve_legacy_key_path(),
        fresh_mnemonic: None,
        mnemonic: payload.mnemonic.clone(),
    })
}

pub fn create_vault(password: &str) -> Result<LoadedWallet> {
    let mnemonic = Keypair::generate_mnemonic()?;
    let phrase = mnemonic.to_string();
    let key = Keypair::from_mnemonic(&mnemonic, "")?;
    let vault_path = resolve_vault_path();
    if let Some(parent) = vault_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let vault = VaultFile::encrypt_mnemonic(&phrase, password, &key.address().to_string())?;
    mesh_crypto::write_secret_file(&vault_path, vault.to_json_pretty()?.as_bytes())?;
    Ok(LoadedWallet {
        key,
        kind: WalletKind::Vault,
        vault_path,
        legacy_key_path: resolve_legacy_key_path(),
        fresh_mnemonic: Some(phrase.clone()),
        mnemonic: Some(phrase),
    })
}

pub fn restore_vault(phrase: &str, password: &str) -> Result<LoadedWallet> {
    let mnemonic = Keypair::mnemonic_from_phrase(phrase)?;
    let key = Keypair::from_mnemonic(&mnemonic, "")?;
    let vault_path = resolve_vault_path();
    if let Some(parent) = vault_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let vault = VaultFile::encrypt_mnemonic(
        &mnemonic.to_string(),
        password,
        &key.address().to_string(),
    )?;
    mesh_crypto::write_secret_file(&vault_path, vault.to_json_pretty()?.as_bytes())?;
    Ok(LoadedWallet {
        key,
        kind: WalletKind::Vault,
        vault_path,
        legacy_key_path: resolve_legacy_key_path(),
        fresh_mnemonic: None,
        mnemonic: Some(mnemonic.to_string()),
    })
}

/// Load legacy plaintext hex without password (migration path).
pub fn load_legacy_key() -> Result<LoadedWallet> {
    let path = resolve_legacy_key_path();
    let hex = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let key = Keypair::from_hex(hex.trim())?;
    Ok(LoadedWallet {
        key,
        kind: WalletKind::LegacyHex,
        vault_path: resolve_vault_path(),
        legacy_key_path: path,
        fresh_mnemonic: None,
        mnemonic: None,
    })
}

pub fn migrate_legacy_to_vault(password: &str) -> Result<LoadedWallet> {
    let legacy = load_legacy_key()?;
    // Cannot recover a mnemonic from a raw key — wrap the secret, and create a NEW mnemonic wallet
    // only when user chooses "create". For migrate we encrypt the legacy secret.
    let vault = VaultFile::encrypt_legacy_key(
        &legacy.key.to_hex(),
        password,
        &legacy.key.address().to_string(),
    )?;
    if let Some(parent) = legacy.vault_path.parent() {
        fs::create_dir_all(parent)?;
    }
    mesh_crypto::write_secret_file(&legacy.vault_path, vault.to_json_pretty()?.as_bytes())?;
    Ok(LoadedWallet {
        key: legacy.key,
        kind: WalletKind::Vault,
        vault_path: legacy.vault_path,
        legacy_key_path: legacy.legacy_key_path,
        fresh_mnemonic: None,
        mnemonic: None,
    })
}

/// Read mnemonic from an unlocked vault for backup display.
pub fn reveal_mnemonic(password: &str) -> Result<String> {
    let vault_path = resolve_vault_path();
    let raw = fs::read_to_string(&vault_path)?;
    let vault = VaultFile::from_json(&raw)?;
    let payload = vault.unlock(password)?;
    payload
        .mnemonic
        .clone()
        .ok_or_else(|| anyhow::anyhow!("this vault was migrated from a legacy key and has no seed phrase — create a new seed wallet to get BIP39 recovery"))
}

pub fn load_or_create_key(path: &Path) -> Result<Keypair> {
    // Compatibility helper for file-picker / legacy paths.
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        bail!("encrypted vault selected — unlock from the Security page instead");
    }
    if path.exists() {
        let hex = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        return Keypair::from_hex(hex.trim()).context("parse wallet key");
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let kp = Keypair::generate();
    fs::write(path, kp.to_hex())?;
    Ok(kp)
}

fn config_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?;
    if bin.join("config.json").exists() {
        return Some(bin.to_path_buf());
    }
    let parent = bin.parent()?;
    if parent.join("config.json").exists() {
        return Some(parent.to_path_buf());
    }
    Some(bin.to_path_buf())
}

fn load_config() -> Option<WalletConfig> {
    let dir = config_dir()?;
    let path = dir.join("config.json");
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}
