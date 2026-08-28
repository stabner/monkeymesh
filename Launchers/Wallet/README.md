# MonkeyMesh Wallet

Double-click **Start-Wallet.bat** — GUI only (no console).

## Security model

- **BIP39 24-word seed** on Create / Restore
- Encrypted vault: **Argon2id + ChaCha20-Poly1305** (`data/wallet.vault.json`)
- Key derivation: **SLIP-0010 Ed25519** `m/44'/999778'/0'/0'/0'`
- Unlock with password; Lock from Network page
- Security page: reveal / back up seed (password required)

Legacy plaintext `wallet.key` can still be opened once, then migrated into a vault.

## Prerequisites

1. `.\Launchers\build-release.ps1`
2. Seed node reachable (default `seednode.hashmonkeys.cloud:18080`)

## Config (`config.json`)

| Key | Meaning |
|-----|---------|
| `rpc` | Node REST URL (default official seed) |
| `wallet_vault` | Encrypted vault path |
| `wallet_key` | Legacy key path (migration only) |

`Start-Wallet.ps1` uses public DNS `http://seednode.hashmonkeys.cloud:18080` (see `Launchers/network.json`).
