# MonkeyMesh Wallet

**Status: ACTIVE** — production wallet is **egui** (`apps/mesh-wallet-gui`, staged as `mesh-wallet.exe`). Tauri frontend under `apps/mesh-wallet/` is **legacy**.

Platforms:
- Windows
- Ubuntu
- macOS (lab)

Features:
- Send/Receive MESH
- Governance voting (RPC)
- Node / tip status
- Miner integration (RPC mine)
- Reward tracking
- AI marketplace access — **shelved** (N12); use seed AI board / explorer instead

## Why this cryptography?

MESH addresses are **Ed25519** (not Bitcoin’s secp256k1). The vault uses the usual industry recovery pattern, with the Ed25519-compatible HD standard:

| Piece | Why |
|-------|-----|
| **BIP39 (24 words)** | Human-readable backup of 256-bit entropy. Same UX as hardware wallets; write offline, never screenshot. |
| **SLIP-0010** | Hierarchical derivation for Ed25519. Classic BIP32/BIP44 CKD does **not** apply cleanly to Ed25519; SLIP-0010 is the SatoshiLabs standard for that. |
| **Path `m/44'/999778'/0'/0'/N'`** | BIP44-shaped path. `999778` is MESH’s **provisional** coin type (private-use until registered with SLIP-0044). `N` indexes HD receive addresses. |
| **Argon2id + XChaCha20-Poly1305 (v2)** | Password-based encryption of the mnemonic on disk. RFC 9106 Argon2id (64 MiB, t=3, p=4) resists GPU/ASIC cracking; XChaCha20-Poly1305 uses a 192-bit random nonce (safe vs 96-bit birthday bound). Passphrase floor is NIST SP 800-63B-4: 15 characters, no composition rules. v1 vaults still unlock. The raw spending key is derived on unlock and not written. |

Legacy plaintext hex key files still open for migration; new wallets should always be BIP39 vaults.
