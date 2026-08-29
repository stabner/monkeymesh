use mesh_types::{Address, Amount, Block, Hash, OutPoint, Transaction, TxId, Utxo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::ChainError;

pub const MAX_MEMPOOL_TXS: usize = 5_000;
/// Anti-Sybil v0: max pending node-market weight per address before next coinbase (Build/06).
pub const MAX_PENDING_NODE_WEIGHT: u64 = 25_000;
/// Anti-Sybil v0: max pending GPU-market weight per address.
pub const MAX_PENDING_GPU_WEIGHT: u64 = 50_000;
/// Cap a single credit event so one gossip flood cannot dominate.
pub const MAX_CREDIT_PER_EVENT: u64 = 500;
/// Minimum liquid/locked MESH (atomic) for a node-market bond (0.1 MESH).
pub const MIN_NODE_BOND_ATOMIC: u64 = 10_000_000;
/// Blocks after unbond request before locked UTXOs become spendable again.
pub const BOND_UNLOCK_COOLDOWN_BLOCKS: u64 = 120;

const WAL_MAGIC: &[u8; 8] = b"MMBK\x01\x00\x00\x00";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipSnapshot {
    pub height: u64,
    pub tip: String,
    pub blocks: usize,
    pub utxos: usize,
}

/// Cold-prune policy / result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColdPrunePlan {
    /// Keep full blocks from this height (inclusive) in the hot WAL.
    pub keep_from_height: u64,
    pub tip_height: u64,
    pub tip: String,
    pub utxo_count: usize,
    pub note: String,
}

/// Minimum hot window so coinbase maturity + retarget still work.
pub const MIN_COLD_PRUNE_KEEP: u64 = 128;

/// On-disk UTXO checkpoint for pruned WALs (`*.utxo.ckpt`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UtxoCheckpoint {
    tip_height: u64,
    tip: String,
    genesis: String,
    keep_from_height: u64,
    /// (txid_hex, vout, address, atomic)
    utxos: Vec<(String, u32, String, u64)>,
}

/// One UTXO frozen as node-market bond collateral.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedBondUtxo {
    pub txid_hex: String,
    pub vout: u32,
    pub atomic: u64,
}

/// Registered node-market bond (Build/06 / Build/27 N5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeBondRec {
    pub peer_id: String,
    pub bonded_at_height: u64,
    /// Locked stake (or legacy soft balance snapshot).
    pub stake_atomic: u64,
    /// UTXOs frozen in place (not moved); excluded from spends.
    #[serde(default)]
    pub locked: Vec<LockedBondUtxo>,
    /// When non-zero and tip ≥ this, locks may be cleared via finalize_unbond.
    #[serde(default)]
    pub unlock_after_height: u64,
    /// Permanently frozen / ineligible after slash.
    #[serde(default)]
    pub slashed: bool,
    /// Soft accounting: locked stake assigned to [`crate::deferred_slash_vault`] (UTXOs stay frozen until on-chain settle).
    #[serde(default)]
    pub slashed_to_vault_atomic: u64,
    /// Tip height when slash was recorded (0 if never).
    #[serde(default)]
    pub slashed_at_height: u64,
}

impl NodeBondRec {
    pub fn locked_atomic(&self) -> u64 {
        self.locked.iter().map(|l| l.atomic).sum()
    }
}

/// Soft-only bond layout (pre-lock) for bincode migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NodeBondRecSoft {
    peer_id: String,
    bonded_at_height: u64,
    stake_atomic: u64,
}

impl From<NodeBondRecSoft> for NodeBondRec {
    fn from(b: NodeBondRecSoft) -> Self {
        Self {
            peer_id: b.peer_id,
            bonded_at_height: b.bonded_at_height,
            stake_atomic: b.stake_atomic,
            locked: Vec::new(),
            unlock_after_height: 0,
            slashed: false,
            slashed_to_vault_atomic: 0,
            slashed_at_height: 0,
        }
    }
}

/// Locked-bond layout before slash-vault accounting fields (bincode migration).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NodeBondRecLocked {
    peer_id: String,
    bonded_at_height: u64,
    stake_atomic: u64,
    #[serde(default)]
    locked: Vec<LockedBondUtxo>,
    #[serde(default)]
    unlock_after_height: u64,
    #[serde(default)]
    slashed: bool,
}

impl From<NodeBondRecLocked> for NodeBondRec {
    fn from(b: NodeBondRecLocked) -> Self {
        let vaulted = if b.slashed {
            b.locked.iter().map(|l| l.atomic).sum::<u64>().max(b.stake_atomic)
        } else {
            0
        };
        Self {
            peer_id: b.peer_id,
            bonded_at_height: b.bonded_at_height,
            stake_atomic: b.stake_atomic,
            locked: b.locked,
            unlock_after_height: b.unlock_after_height,
            slashed: b.slashed,
            slashed_to_vault_atomic: vaulted,
            slashed_at_height: 0,
        }
    }
}

/// Soft state persisted without the block list (Build/27 N2 / Build/30).
use crate::meta_migrate::{deserialize_meta, ChainMeta};

#[derive(Default, Serialize, Deserialize)]
struct ChainData {
    blocks: Vec<Block>,
    mempool: Vec<Transaction>,
    /// Pending GPU market weights (address hex → weight). Cleared when paid in coinbase.
    #[serde(default)]
    gpu_scores: HashMap<String, u64>,
    /// Pending node market weights.
    #[serde(default)]
    node_scores: HashMap<String, u64>,
    /// Recent verified AI receipts (telemetry / audit; capped).
    #[serde(default)]
    ai_receipts: Vec<mesh_types::AiJobReceipt>,
    /// Pending / historical adaptive proposals (AI propose-only until activated).
    #[serde(default)]
    proposals: Vec<mesh_types::ParamProposal>,
    /// Human-activated / auto-applied soft envelopes (control plane).
    #[serde(default)]
    active_envelopes: mesh_types::ProtocolEnvelopes,
    #[serde(default)]
    next_proposal_id: u64,
    /// Height when soft envelopes were last auto-applied.
    #[serde(default)]
    last_auto_adapt_at_height: u64,
    /// Proposal id of last soft auto-apply (empty if never).
    #[serde(default)]
    last_auto_adapt_proposal_id: String,
    /// ProtocolEval receipt count at last soft auto-apply.
    #[serde(default)]
    last_auto_adapt_eval_count: u64,
    /// Monotonic soft-envelope epoch (not a tip fork).
    #[serde(default)]
    param_epoch: u64,
    /// Recent param epoch history (capped).
    #[serde(default)]
    epoch_history: Vec<mesh_types::ParamEpoch>,
    #[serde(default)]
    node_bonds: HashMap<String, NodeBondRec>,
    /// Grover certificate count at last quantum-gated retarget adapt (Build/30).
    #[serde(default)]
    last_retarget_adapt_grover_count: u64,
    /// Recent improvement certificates (capped ring).
    #[serde(default)]
    improvement_certs: Vec<mesh_types::ImprovementCertificate>,
}

impl ChainData {
    fn from_meta(meta: ChainMeta, blocks: Vec<Block>) -> Self {
        Self {
            blocks,
            mempool: meta.mempool,
            gpu_scores: meta.gpu_scores,
            node_scores: meta.node_scores,
            ai_receipts: meta.ai_receipts,
            proposals: meta.proposals,
            active_envelopes: meta.active_envelopes,
            next_proposal_id: meta.next_proposal_id,
            last_auto_adapt_at_height: meta.last_auto_adapt_at_height,
            last_auto_adapt_proposal_id: meta.last_auto_adapt_proposal_id,
            last_auto_adapt_eval_count: meta.last_auto_adapt_eval_count,
            param_epoch: meta.param_epoch,
            epoch_history: meta.epoch_history,
            node_bonds: meta.node_bonds,
            last_retarget_adapt_grover_count: meta.last_retarget_adapt_grover_count,
            improvement_certs: meta.improvement_certs,
        }
    }

    fn to_meta(&self) -> ChainMeta {
        ChainMeta {
            mempool: self.mempool.clone(),
            gpu_scores: self.gpu_scores.clone(),
            node_scores: self.node_scores.clone(),
            ai_receipts: self.ai_receipts.clone(),
            proposals: self.proposals.clone(),
            active_envelopes: self.active_envelopes.clone(),
            next_proposal_id: self.next_proposal_id,
            last_auto_adapt_at_height: self.last_auto_adapt_at_height,
            last_auto_adapt_proposal_id: self.last_auto_adapt_proposal_id.clone(),
            last_auto_adapt_eval_count: self.last_auto_adapt_eval_count,
            param_epoch: self.param_epoch,
            epoch_history: self.epoch_history.clone(),
            node_bonds: self.node_bonds.clone(),
            last_retarget_adapt_grover_count: self.last_retarget_adapt_grover_count,
            improvement_certs: self.improvement_certs.clone(),
        }
    }
}

pub struct ChainStore {
    path: PathBuf,
    data: ChainData,
    utxos: HashMap<OutPoint, Utxo>,
    /// Soft scores / telemetry pending disk write (blocks always append to WAL).
    dirty: bool,
    /// Genesis block id (survives cold prune when height 0 is dropped).
    genesis_hash: Hash,
    /// Hot WAL starts above genesis (checkpoint present).
    pruned: bool,
    /// Bumps when gpu/node pending scores change (template-cache key; not persisted).
    scores_epoch: u64,
    /// Recent service attestations (runtime ring; not consensus-critical).
    service_log: std::collections::VecDeque<mesh_types::NodeServiceAttestation>,
    /// Preferred on-chain slash settle txid (hex) from SlashMark gossip — runtime only.
    pending_slash_settles: HashMap<String, String>,
    /// Replica IBD: skip per-block snap/ckpt/fsync until flushed.
    bulk_import: bool,
}

impl ChainStore {
    fn meta_path(path: &Path) -> PathBuf {
        path.with_extension("meta.bin")
    }

    fn wal_path(path: &Path) -> PathBuf {
        path.with_extension("blocks.wal")
    }

    fn utxo_ckpt_path(path: &Path) -> PathBuf {
        path.with_extension("utxo.ckpt")
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, ChainError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let meta_path = Self::meta_path(&path);
        let wal_path = Self::wal_path(&path);

        let data = if meta_path.exists() || wal_path.exists() {
            let meta = if meta_path.exists() {
                let meta_bytes = fs::read(&meta_path)?;
                match deserialize_meta(&meta_bytes) {
                    Ok(m) => m,
                    Err(e) => {
                        let stamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let bak = meta_path.with_extension(format!("meta.bin.corrupt-{stamp}"));
                        let _ = fs::rename(&meta_path, &bak);
                        eprintln!(
                            "mesh-chain: corrupt meta ({e}); moved to {} — keeping blocks.wal, soft state reset",
                            bak.display()
                        );
                        ChainMeta::default()
                    }
                }
            } else {
                ChainMeta::default()
            };
            let blocks = if wal_path.exists() {
                match read_blocks_wal(&wal_path) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("mesh-chain: wal read failed ({e}); starting empty blocks");
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };
            ChainData::from_meta(meta, blocks)
        } else if path.exists() {
            // Legacy monolithic chain.bin → split into WAL + meta once.
            let bytes = fs::read(&path)?;
            match bincode::deserialize::<ChainData>(&bytes) {
                Ok(d) => {
                    migrate_monolithic(&path, &d)?;
                    d
                }
                Err(e) => {
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let bak = path.with_extension(format!("bin.incompat-{stamp}"));
                    let _ = fs::rename(&path, &bak);
                    eprintln!(
                        "mesh-chain: incompatible chain store ({e}); moved to {} and starting empty",
                        bak.display()
                    );
                    ChainData::default()
                }
            }
        } else {
            ChainData::default()
        };

        let mut store = Self {
            path,
            data,
            utxos: HashMap::new(),
            dirty: false,
            genesis_hash: Hash::zero(),
            pruned: false,
            scores_epoch: 0,
            service_log: std::collections::VecDeque::with_capacity(64),
            pending_slash_settles: HashMap::new(),
            bulk_import: false,
        };
        store.rebuild_utxos()?;
        Ok(store)
    }

    fn reindex(&mut self) -> Result<(), ChainError> {
        self.utxos.clear();
        for block in &self.data.blocks {
            apply_block_utxos(&mut self.utxos, block)?;
        }
        if let Some(g) = self.get_by_height(0) {
            self.genesis_hash = g.id();
        }
        Ok(())
    }

    /// Rebuild UTXOs from checkpoint (+ delta) or full WAL reindex.
    fn rebuild_utxos(&mut self) -> Result<(), ChainError> {
        if let Some(ckpt) = self.load_utxo_checkpoint()? {
            self.genesis_hash = Hash::from_hex(&ckpt.genesis).unwrap_or_else(|_| Hash::zero());
            self.pruned = ckpt.keep_from_height > 0;
            let tip = self.data.blocks.last();
            match tip {
                Some(tip) if tip.header.height == ckpt.tip_height && tip.id().to_string() == ckpt.tip => {
                    self.utxos = utxo_ckpt_to_map(&ckpt)?;
                    return Ok(());
                }
                Some(tip) if tip.header.height > ckpt.tip_height => {
                    self.utxos = utxo_ckpt_to_map(&ckpt)?;
                    for b in &self.data.blocks {
                        if b.header.height > ckpt.tip_height {
                            apply_block_utxos(&mut self.utxos, b)?;
                        }
                    }
                    return Ok(());
                }
                _ => {
                    // Checkpoint ahead of WAL or mismatch — fall through to full reindex if possible.
                    eprintln!(
                        "mesh-chain: utxo checkpoint tip mismatch; attempting WAL reindex"
                    );
                }
            }
        }
        self.pruned = false;
        self.reindex()?;
        Ok(())
    }

    /// Persist soft state only (scores, mempool, receipts, gov) — not the block list.
    pub fn persist(&mut self) -> Result<(), ChainError> {
        self.persist_meta()?;
        self.dirty = false;
        let _ = self.write_tip_snapshot_file();
        Ok(())
    }

    fn persist_meta(&self) -> Result<(), ChainError> {
        let meta = self.data.to_meta();
        let bytes = bincode::serialize(&meta).map_err(|e| ChainError::Store(e.to_string()))?;
        let meta_path = Self::meta_path(&self.path);
        let tmp = meta_path.with_extension("meta.bin.tmp");
        {
            use std::io::Write;
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            if wal_fsync_enabled() {
                f.sync_all()?;
            } else {
                f.flush()?;
            }
        }
        fs::rename(&tmp, &meta_path)?;
        Ok(())
    }

    /// Flush soft-score / telemetry dirty flag without rewriting blocks.
    pub fn flush_if_dirty(&mut self) -> Result<(), ChainError> {
        if self.dirty {
            self.persist()?;
        }
        Ok(())
    }

    fn touch(&mut self) {
        self.dirty = true;
    }

    /// Lightweight tip meta for catch-up / edge health (Build/10 SNAPSHOT v0).
    pub fn tip_snapshot(&self) -> TipSnapshot {
        TipSnapshot {
            height: self.height(),
            tip: self.tip_hash().to_string(),
            blocks: self.len(),
            utxos: self.utxos.len(),
        }
    }

    /// Cold-prune plan: keep last `keep_blocks` bodies; UTXO set is the checkpoint.
    pub fn cold_prune_plan(&self, keep_blocks: u64) -> ColdPrunePlan {
        let tip_height = self.height();
        let keep = keep_blocks.max(MIN_COLD_PRUNE_KEEP);
        let keep_from = tip_height.saturating_sub(keep.saturating_sub(1));
        ColdPrunePlan {
            keep_from_height: keep_from,
            tip_height,
            tip: self.tip_hash().to_string(),
            utxo_count: self.utxos.len(),
            note: if self.pruned {
                "already pruned — re-apply shrinks hot WAL further if keep_from rises".into()
            } else {
                "POST /v1/snapshot/prune with confirm=1 and MESH_COLD_PRUNE=1 to apply".into()
            },
        }
    }

    pub fn genesis_hash(&self) -> Hash {
        if self.genesis_hash != Hash::zero() {
            return self.genesis_hash;
        }
        self.get_by_height(0)
            .map(|b| b.id())
            .unwrap_or_else(Hash::zero)
    }

    pub fn is_pruned(&self) -> bool {
        self.pruned
    }

    pub fn hot_from_height(&self) -> u64 {
        self.data
            .blocks
            .first()
            .map(|b| b.header.height)
            .unwrap_or(0)
    }

    fn load_utxo_checkpoint(&self) -> Result<Option<UtxoCheckpoint>, ChainError> {
        let path = Self::utxo_ckpt_path(&self.path);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path)?;
        let ckpt: UtxoCheckpoint =
            bincode::deserialize(&bytes).map_err(|e| ChainError::Store(e.to_string()))?;
        Ok(Some(ckpt))
    }

    fn write_utxo_checkpoint(&self, keep_from_height: u64) -> Result<(), ChainError> {
        let tip = self.tip().ok_or_else(|| ChainError::Store("no tip".into()))?;
        let genesis = self.genesis_hash();
        if genesis == Hash::zero() {
            return Err(ChainError::Store("genesis hash unknown".into()));
        }
        let mut utxos: Vec<_> = self
            .utxos
            .iter()
            .map(|(op, u)| {
                (
                    op.txid.to_hex(),
                    op.vout,
                    u.address.to_string(),
                    u.amount.atomic(),
                )
            })
            .collect();
        utxos.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let ckpt = UtxoCheckpoint {
            tip_height: tip.header.height,
            tip: tip.id().to_string(),
            genesis: genesis.to_hex(),
            keep_from_height,
            utxos,
        };
        let bytes = bincode::serialize(&ckpt).map_err(|e| ChainError::Store(e.to_string()))?;
        let path = Self::utxo_ckpt_path(&self.path);
        let tmp = path.with_extension("utxo.ckpt.tmp");
        {
            use std::io::Write;
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            if wal_fsync_enabled() {
                f.sync_all()?;
            } else {
                f.flush()?;
            }
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Truncate WAL to hot window and persist UTXO checkpoint. Irreversible for local bodies.
    pub fn apply_cold_prune(&mut self, keep_blocks: u64) -> Result<ColdPrunePlan, ChainError> {
        let keep = keep_blocks.max(MIN_COLD_PRUNE_KEEP);
        let tip_height = self.height();
        if self.data.blocks.is_empty() {
            return Err(ChainError::Store("empty chain".into()));
        }
        if let Some(g) = self.get_by_height(0) {
            self.genesis_hash = g.id();
        } else if self.genesis_hash == Hash::zero() {
            return Err(ChainError::Store(
                "cannot prune: genesis hash unknown (open full history once first)".into(),
            ));
        }
        let keep_from = tip_height.saturating_sub(keep.saturating_sub(1));
        let before = self.data.blocks.len();
        if keep_from <= self.hot_from_height() && self.data.blocks.len() as u64 <= keep {
            let mut plan = self.cold_prune_plan(keep);
            plan.note = "already within keep window — checkpoint refreshed".into();
            self.write_utxo_checkpoint(self.hot_from_height())?;
            self.pruned = keep_from > 0 || self.hot_from_height() > 0;
            return Ok(plan);
        }
        self.data
            .blocks
            .retain(|b| b.header.height >= keep_from);
        write_blocks_wal(&Self::wal_path(&self.path), &self.data.blocks)?;
        self.write_utxo_checkpoint(keep_from)?;
        self.persist_meta()?;
        let _ = self.write_tip_snapshot_file();
        self.pruned = keep_from > 0;
        let after = self.data.blocks.len();
        Ok(ColdPrunePlan {
            keep_from_height: keep_from,
            tip_height,
            tip: self.tip_hash().to_string(),
            utxo_count: self.utxos.len(),
            note: format!(
                "applied — hot WAL {} → {} blocks (dropped {}); utxo.ckpt written",
                before,
                after,
                before.saturating_sub(after)
            ),
        })
    }

    /// Compact UTXO export for snapshot clients (capped).
    pub fn utxo_export(&self, offset: usize, limit: usize) -> Vec<(String, u32, String, u64)> {
        let mut rows: Vec<_> = self
            .utxos
            .iter()
            .map(|(op, u)| {
                (
                    op.txid.to_hex(),
                    op.vout,
                    u.address.to_string(),
                    u.amount.atomic(),
                )
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        rows.into_iter().skip(offset).take(limit).collect()
    }

    fn write_tip_snapshot_file(&self) -> Result<(), ChainError> {
        let snap = self.tip_snapshot();
        let path = self.path.with_extension("snap.json");
        let body = serde_json::to_vec_pretty(&snap).map_err(|e| ChainError::Store(e.to_string()))?;
        let tmp = path.with_extension("snap.json.tmp");
        fs::write(&tmp, body)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn append(&mut self, block: &Block) -> Result<(), ChainError> {
        apply_block_utxos(&mut self.utxos, block)?;
        let confirmed: Vec<TxId> = block.txs.iter().map(|t| t.txid()).collect();
        self.data
            .mempool
            .retain(|t| !confirmed.contains(&t.txid()));
        append_block_wal(&Self::wal_path(&self.path), block)?;
        self.data.blocks.push(block.clone());
        if self.genesis_hash == Hash::zero() && block.header.height == 0 {
            self.genesis_hash = block.id();
        }
        let tick = !self.bulk_import || self.height() % 64 == 0;
        if tick {
            self.persist_meta()?;
            let _ = self.write_tip_snapshot_file();
            if self.pruned || Self::utxo_ckpt_path(&self.path).exists() {
                let keep_from = self.hot_from_height();
                let _ = self.write_utxo_checkpoint(keep_from);
            }
        } else {
            self.dirty = true;
        }
        if !self.bulk_import {
            self.maybe_auto_prune()?;
        }
        Ok(())
    }

    /// `MESH_AUTO_PRUNE=1` + `MESH_COLD_PRUNE=1` → trim hot WAL after append.
    fn maybe_auto_prune(&mut self) -> Result<(), ChainError> {
        let auto = std::env::var("MESH_AUTO_PRUNE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let allowed = std::env::var("MESH_COLD_PRUNE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !auto || !allowed {
            return Ok(());
        }
        let keep = std::env::var("MESH_KEEP_BLOCKS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(2_048u64)
            .max(MIN_COLD_PRUNE_KEEP);
        // Only prune when hot window is meaningfully larger than keep.
        let hot = self.data.blocks.len() as u64;
        if hot <= keep.saturating_mul(2) {
            return Ok(());
        }
        match self.apply_cold_prune(keep) {
            Ok(plan) => {
                tracing::info!(
                    keep_from = plan.keep_from_height,
                    tip = plan.tip_height,
                    note = %plan.note,
                    "auto cold-prune applied"
                );
                Ok(())
            }
            Err(e) => {
                tracing::warn!(error = %e, "auto cold-prune skipped");
                Ok(())
            }
        }
    }

    pub fn height(&self) -> u64 {
        self.data
            .blocks
            .last()
            .map(|b| b.header.height)
            .unwrap_or(0)
    }

    pub fn tip(&self) -> Option<Block> {
        self.data.blocks.last().cloned()
    }

    /// Disconnect the tip block (depth-1 reorg). Refuses to pop genesis.
    /// Rebuilds UTXOs from the remaining hot WAL / checkpoint.
    pub fn pop_tip(&mut self) -> Result<Block, ChainError> {
        let tip = self
            .data
            .blocks
            .last()
            .cloned()
            .ok_or_else(|| ChainError::Store("no tip to pop".into()))?;
        if tip.header.height == 0 {
            return Err(ChainError::Store("cannot pop genesis".into()));
        }
        self.data.blocks.pop();
        // Drop checkpoint if it pointed at the removed tip so rebuild uses WAL.
        let ckpt_path = Self::utxo_ckpt_path(&self.path);
        if ckpt_path.exists() {
            if let Ok(Some(ckpt)) = self.load_utxo_checkpoint() {
                if ckpt.tip_height >= tip.header.height || ckpt.tip == tip.id().to_string() {
                    let _ = fs::remove_file(&ckpt_path);
                }
            }
        }
        self.rebuild_utxos()?;
        write_blocks_wal(&Self::wal_path(&self.path), &self.data.blocks)?;
        self.persist_meta()?;
        let _ = self.write_tip_snapshot_file();
        tracing::info!(
            height = tip.header.height,
            id = %tip.id(),
            "popped tip for depth-1 reorg"
        );
        Ok(tip)
    }

    pub fn tip_hash(&self) -> Hash {
        self.tip().map(|b| b.id()).unwrap_or_else(Hash::zero)
    }

    pub fn get_by_hash(&self, id: &Hash) -> Option<Block> {
        self.data
            .blocks
            .iter()
            .rev()
            .find(|b| b.id() == *id)
            .cloned()
    }

    pub fn get_by_height(&self, height: u64) -> Option<Block> {
        let hot_from = self.hot_from_height();
        if height < hot_from {
            return None;
        }
        let idx = (height - hot_from) as usize;
        match self.data.blocks.get(idx) {
            Some(b) if b.header.height == height => Some(b.clone()),
            // Fallback for sparse/legacy layouts (should be rare).
            _ => self
                .data
                .blocks
                .iter()
                .find(|b| b.header.height == height)
                .cloned(),
        }
    }

    pub fn len(&self) -> usize {
        self.data.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.blocks.is_empty()
    }

    pub fn set_bulk_import(&mut self, bulk: bool) {
        self.bulk_import = bulk;
        if !bulk {
            let _ = self.persist_meta();
            let _ = self.write_tip_snapshot_file();
            if self.pruned || Self::utxo_ckpt_path(&self.path).exists() {
                let keep_from = self.hot_from_height();
                let _ = self.write_utxo_checkpoint(keep_from);
            }
            self.dirty = false;
        }
    }

    /// Load an official seed UTXO snapshot + hot WAL tail (pruned replica).
    /// UTXOs must already be at `hot_blocks` tip — bodies are not replayed onto the set.
    pub fn install_official_prune(
        &mut self,
        genesis: Hash,
        utxos: HashMap<OutPoint, Utxo>,
        hot_blocks: Vec<Block>,
    ) -> Result<(), ChainError> {
        if hot_blocks.is_empty() {
            return Err(ChainError::Store("official snapshot missing hot blocks".into()));
        }
        for pair in hot_blocks.windows(2) {
            if pair[1].header.height != pair[0].header.height + 1 {
                return Err(ChainError::Store("official snapshot heights not consecutive".into()));
            }
            if pair[1].header.prev_hash != pair[0].id() {
                return Err(ChainError::Store("official snapshot prev-hash break".into()));
            }
        }
        if genesis == Hash::zero() {
            return Err(ChainError::Store("official snapshot genesis missing".into()));
        }
        if utxos.is_empty() {
            return Err(ChainError::Store("official snapshot UTXO set empty".into()));
        }
        self.genesis_hash = genesis;
        self.utxos = utxos;
        self.data.blocks = hot_blocks;
        self.pruned = true;
        self.bulk_import = false;
        write_blocks_wal(&Self::wal_path(&self.path), &self.data.blocks)?;
        self.write_utxo_checkpoint(self.hot_from_height())?;
        self.persist_meta()?;
        let _ = self.write_tip_snapshot_file();
        self.dirty = false;
        Ok(())
    }

    pub fn utxos(&self) -> &HashMap<OutPoint, Utxo> {
        &self.utxos
    }

    #[cfg(test)]
    pub fn test_inject_utxo(&mut self, op: OutPoint, utxo: Utxo) {
        self.utxos.insert(op, utxo);
    }

    #[cfg(test)]
    pub fn test_insert_bond(&mut self, address: &Address, rec: NodeBondRec) {
        self.data.node_bonds.insert(address.to_hex(), rec);
        self.touch();
    }

    pub fn utxos_for(&self, address: &Address) -> Vec<(OutPoint, Utxo)> {
        self.utxos
            .iter()
            .filter(|(_, u)| u.address == *address)
            .map(|(op, u)| (*op, u.clone()))
            .collect()
    }

    /// UTXOs that are not frozen by a node bond.
    pub fn spendable_utxos_for(&self, address: &Address) -> Vec<(OutPoint, Utxo)> {
        self.utxos_for(address)
            .into_iter()
            .filter(|(op, _)| !self.is_outpoint_locked(op))
            .collect()
    }

    pub fn spendable_balance(&self, address: &Address) -> Amount {
        self.spendable_utxos_for(address)
            .into_iter()
            .fold(Amount::ZERO, |acc, (_, u)| {
                acc.checked_add(u.amount).unwrap_or(acc)
            })
    }

    pub fn is_outpoint_locked(&self, op: &OutPoint) -> bool {
        let txid = op.txid.to_hex();
        // Active locks and slashed collateral both freeze outs (slash = permanent).
        self.data.node_bonds.values().any(|b| {
            b.locked
                .iter()
                .any(|l| l.txid_hex == txid && l.vout == op.vout)
        })
    }

    pub fn balance(&self, address: &Address) -> Amount {
        self.utxos
            .values()
            .filter(|u| u.address == *address)
            .fold(Amount::ZERO, |acc, u| acc.checked_add(u.amount).unwrap_or(acc))
    }

    pub fn mempool(&self) -> &[Transaction] {
        &self.data.mempool
    }

    pub fn mempool_push(&mut self, tx: Transaction) -> Result<(), ChainError> {
        if self.data.mempool.len() >= MAX_MEMPOOL_TXS {
            return Err(ChainError::InvalidTx("mempool full".into()));
        }
        if tx.memo.len() > crate::MAX_MEMO_BYTES {
            return Err(ChainError::InvalidTx("memo too large".into()));
        }
        if self.data.mempool.iter().any(|t| t.txid() == tx.txid()) {
            return Err(ChainError::InvalidTx("duplicate mempool tx".into()));
        }
        self.data.mempool.push(tx);
        // Soft persist — flushed with scores / research tick (N2).
        self.touch();
        Ok(())
    }

    /// Drop mempool txs that spend any of `tx`'s inputs (used when preferred settle replaces a race).
    pub fn mempool_evict_input_conflicts(&mut self, tx: &Transaction) -> usize {
        let spend: std::collections::HashSet<(Hash, u32)> = tx
            .inputs
            .iter()
            .map(|i| (i.prev_txid, i.vout))
            .collect();
        let before = self.data.mempool.len();
        self.data.mempool.retain(|existing| {
            if existing.txid() == tx.txid() {
                return true;
            }
            !existing
                .inputs
                .iter()
                .any(|e| spend.contains(&(e.prev_txid, e.vout)))
        });
        let removed = before.saturating_sub(self.data.mempool.len());
        if removed > 0 {
            self.touch();
        }
        removed
    }

    pub fn set_preferred_slash_settle(&mut self, address: &Address, txid_hex: &str) {
        let t = txid_hex.trim();
        if t.is_empty() {
            return;
        }
        self.pending_slash_settles
            .insert(address.to_hex(), t.to_string());
    }

    pub fn preferred_slash_settle(&self, address: &Address) -> Option<&str> {
        self.pending_slash_settles
            .get(&address.to_hex())
            .map(|s| s.as_str())
    }

    pub fn clear_preferred_slash_settle(&mut self, address: &Address) {
        self.pending_slash_settles.remove(&address.to_hex());
    }

    /// First mempool tx that shares an input with `tx` (or same txid).
    pub fn mempool_input_conflict(&self, tx: &Transaction) -> Option<&Transaction> {
        for existing in &self.data.mempool {
            if existing.txid() == tx.txid() {
                return Some(existing);
            }
            for inp in &tx.inputs {
                if existing
                    .inputs
                    .iter()
                    .any(|e| e.prev_txid == inp.prev_txid && e.vout == inp.vout)
                {
                    return Some(existing);
                }
            }
        }
        None
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Delete WAL / meta / snap / utxo checkpoint for this store path.
    /// Safe if files are missing. Caller must drop any open `Chain` first.
    pub fn wipe_files(path: impl AsRef<Path>) {
        let path = path.as_ref();
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(Self::meta_path(path));
        let _ = fs::remove_file(Self::wal_path(path));
        let _ = fs::remove_file(Self::utxo_ckpt_path(path));
        let _ = fs::remove_file(path.with_extension("snap.json"));
        let _ = fs::remove_file(path.with_extension("finality.json"));
        let _ = fs::remove_file(path.with_extension("bin.monolithic-bak"));
    }

    pub fn utxo_snapshot(&self) -> HashMap<OutPoint, Utxo> {
        self.utxos.clone()
    }

    pub fn gpu_scores(&self) -> &HashMap<String, u64> {
        &self.data.gpu_scores
    }

    pub fn node_scores(&self) -> &HashMap<String, u64> {
        &self.data.node_scores
    }

    pub fn ai_receipts(&self) -> &[mesh_types::AiJobReceipt] {
        &self.data.ai_receipts
    }

    pub fn credit_gpu(&mut self, address: &Address, weight: u64) -> Result<(), ChainError> {
        let min_w = self.data.active_envelopes.min_verifier_weight;
        if weight < min_w {
            return Ok(());
        }
        let add = weight.min(MAX_CREDIT_PER_EVENT);
        let key = address.to_hex();
        let e = self.data.gpu_scores.entry(key).or_insert(0);
        let room = MAX_PENDING_GPU_WEIGHT.saturating_sub(*e);
        let add = add.min(room);
        if add == 0 {
            return Ok(());
        }
        *e = e.saturating_add(add);
        self.scores_epoch = self.scores_epoch.saturating_add(1);
        self.touch();
        Ok(())
    }

    pub fn credit_node(&mut self, address: &Address, weight: u64) -> Result<(), ChainError> {
        if weight == 0 {
            return Ok(());
        }
        if node_bond_required() && !self.is_node_bond_eligible(address) {
            return Ok(());
        }
        let add = weight.min(MAX_CREDIT_PER_EVENT);
        let key = address.to_hex();
        let e = self.data.node_scores.entry(key).or_insert(0);
        let room = MAX_PENDING_NODE_WEIGHT.saturating_sub(*e);
        let add = add.min(room);
        if add == 0 {
            return Ok(());
        }
        *e = e.saturating_add(add);
        self.scores_epoch = self.scores_epoch.saturating_add(1);
        self.touch();
        Ok(())
    }

    pub fn scores_epoch(&self) -> u64 {
        self.scores_epoch
    }

    pub fn push_service_attestation(&mut self, att: mesh_types::NodeServiceAttestation) {
        const CAP: usize = 64;
        self.service_log.push_back(att);
        while self.service_log.len() > CAP {
            self.service_log.pop_front();
        }
    }

    pub fn service_attestations(&self) -> impl Iterator<Item = &mesh_types::NodeServiceAttestation> {
        self.service_log.iter()
    }

    pub fn node_bonds(&self) -> &HashMap<String, NodeBondRec> {
        &self.data.node_bonds
    }

    pub fn bond_for(&self, address: &Address) -> Option<&NodeBondRec> {
        self.data.node_bonds.get(&address.to_hex())
    }

    /// Bonded with locked collateral (preferred) or legacy soft stake.
    pub fn is_node_bond_eligible(&self, address: &Address) -> bool {
        let Some(b) = self.bond_for(address) else {
            return false;
        };
        if b.slashed {
            return false;
        }
        if b.unlock_after_height > 0 {
            return false; // unlocking
        }
        let locked = b.locked_atomic();
        if locked > 0 {
            // Ensure locked outs still exist.
            let live: u64 = b
                .locked
                .iter()
                .filter(|l| {
                    Hash::from_hex(&l.txid_hex)
                        .ok()
                        .and_then(|txid| {
                            self.utxos
                                .get(&OutPoint::new(txid, l.vout))
                                .map(|u| u.address == *address && u.amount.atomic() == l.atomic)
                        })
                        .unwrap_or(false)
                })
                .map(|l| l.atomic)
                .sum();
            return live >= MIN_NODE_BOND_ATOMIC;
        }
        // Legacy soft bond: liquid balance gate.
        self.balance(address).atomic() >= MIN_NODE_BOND_ATOMIC
    }

    /// Register bond by freezing UTXOs in place (≥ min stake).
    pub fn register_node_bond(
        &mut self,
        address: &Address,
        peer_id: &str,
        at_height: u64,
    ) -> Result<NodeBondRec, ChainError> {
        if let Some(existing) = self.bond_for(address).cloned() {
            if existing.slashed {
                return Err(ChainError::InvalidTx("bond slashed — cannot re-register".into()));
            }
            if existing.locked_atomic() >= MIN_NODE_BOND_ATOMIC && existing.unlock_after_height == 0
            {
                let mut rec = existing;
                let peer_id = peer_id.trim().to_string();
                if !peer_id.is_empty() {
                    for (addr, b) in &self.data.node_bonds {
                        if b.peer_id == peer_id && addr != &address.to_hex() {
                            return Err(ChainError::InvalidTx(
                                "peer_id already bonded to another address".into(),
                            ));
                        }
                    }
                    rec.peer_id = peer_id;
                }
                self.data.node_bonds.insert(address.to_hex(), rec.clone());
                self.touch();
                return Ok(rec);
            }
        }

        let peer_id = peer_id.trim().to_string();
        if !peer_id.is_empty() {
            for (addr, b) in &self.data.node_bonds {
                if b.peer_id == peer_id && addr != &address.to_hex() {
                    return Err(ChainError::InvalidTx(
                        "peer_id already bonded to another address".into(),
                    ));
                }
            }
        }

        let mut owned = self.spendable_utxos_for(address);
        owned.sort_by(|a, b| b.1.amount.atomic().cmp(&a.1.amount.atomic()));
        let mut locked = Vec::new();
        let mut sum = 0u64;
        for (op, u) in owned {
            if sum >= MIN_NODE_BOND_ATOMIC {
                break;
            }
            sum = sum.saturating_add(u.amount.atomic());
            locked.push(LockedBondUtxo {
                txid_hex: op.txid.to_hex(),
                vout: op.vout,
                atomic: u.amount.atomic(),
            });
        }
        if sum < MIN_NODE_BOND_ATOMIC {
            return Err(ChainError::InvalidTx(format!(
                "node bond requires ≥ {} atomic spendable MESH to lock (have {sum})",
                MIN_NODE_BOND_ATOMIC
            )));
        }

        let rec = NodeBondRec {
            peer_id,
            bonded_at_height: at_height,
            stake_atomic: sum,
            locked,
            unlock_after_height: 0,
            slashed: false,
            slashed_to_vault_atomic: 0,
            slashed_at_height: 0,
        };
        self.data.node_bonds.insert(address.to_hex(), rec.clone());
        self.touch();
        Ok(rec)
    }

    /// Begin unlock cooldown; UTXOs stay frozen until [`finalize_node_unbond`].
    pub fn request_node_unbond(
        &mut self,
        address: &Address,
        at_height: u64,
    ) -> Result<NodeBondRec, ChainError> {
        let key = address.to_hex();
        let Some(mut rec) = self.data.node_bonds.get(&key).cloned() else {
            return Err(ChainError::InvalidTx("no bond".into()));
        };
        if rec.slashed {
            return Err(ChainError::InvalidTx("bond slashed".into()));
        }
        rec.unlock_after_height = at_height.saturating_add(BOND_UNLOCK_COOLDOWN_BLOCKS);
        self.data.node_bonds.insert(key, rec.clone());
        self.touch();
        Ok(rec)
    }

    /// Clear locks after cooldown (or immediately if no locks / soft bond).
    pub fn finalize_node_unbond(
        &mut self,
        address: &Address,
        at_height: u64,
    ) -> Result<NodeBondRec, ChainError> {
        let key = address.to_hex();
        let Some(mut rec) = self.data.node_bonds.get(&key).cloned() else {
            return Err(ChainError::InvalidTx("no bond".into()));
        };
        if rec.slashed {
            return Err(ChainError::InvalidTx("bond slashed".into()));
        }
        if rec.unlock_after_height == 0 {
            return Err(ChainError::InvalidTx("unbond not requested".into()));
        }
        if at_height < rec.unlock_after_height {
            return Err(ChainError::InvalidTx(format!(
                "unbond ready at height {}",
                rec.unlock_after_height
            )));
        }
        rec.locked.clear();
        rec.stake_atomic = 0;
        rec.unlock_after_height = 0;
        self.data.node_bonds.insert(key, rec.clone());
        self.touch();
        Ok(rec)
    }

    /// Slash bond: freeze forever, assign locked stake to slash vault (soft accounting),
    /// clear pending node score. UTXOs are not moved on-chain until a future settle rule.
    pub fn slash_node_bond(
        &mut self,
        address: &Address,
        at_height: u64,
    ) -> Result<NodeBondRec, ChainError> {
        let key = address.to_hex();
        let Some(mut rec) = self.data.node_bonds.get(&key).cloned() else {
            return Err(ChainError::InvalidTx("no bond".into()));
        };
        if rec.slashed {
            return Ok(rec);
        }
        let vaulted = rec.locked_atomic().max(rec.stake_atomic);
        rec.slashed = true;
        rec.unlock_after_height = 0;
        rec.slashed_to_vault_atomic = vaulted;
        rec.slashed_at_height = at_height;
        self.data.node_scores.remove(&key);
        self.data.node_bonds.insert(key, rec.clone());
        self.touch();
        Ok(rec)
    }

    /// Soft total assigned to the slash vault across all bonds (not on-chain UTXOs yet).
    pub fn slashed_vault_atomic(&self) -> u64 {
        self.data
            .node_bonds
            .values()
            .map(|b| b.slashed_to_vault_atomic)
            .sum()
    }

    /// Soft slash mark from gossip (freeze eligibility before settle confirms).
    pub fn apply_slash_mark(
        &mut self,
        address: &Address,
        height: u64,
        stake_atomic: u64,
        peer_id: &str,
        preferred_settle_txid: &str,
    ) -> Result<NodeBondRec, ChainError> {
        self.set_preferred_slash_settle(address, preferred_settle_txid);
        if let Some(existing) = self.bond_for(address).cloned() {
            if existing.slashed {
                return Ok(existing);
            }
            return self.slash_node_bond(address, height);
        }
        let rec = NodeBondRec {
            peer_id: peer_id.trim().to_string(),
            bonded_at_height: height,
            stake_atomic,
            locked: Vec::new(),
            unlock_after_height: 0,
            slashed: true,
            slashed_to_vault_atomic: stake_atomic,
            slashed_at_height: height,
        };
        self.data.node_bonds.insert(address.to_hex(), rec.clone());
        self.data.node_scores.remove(&address.to_hex());
        self.touch();
        Ok(rec)
    }

    /// After an on-chain slash settle confirms, clear spent locks from meta.
    pub fn apply_slash_settle_tx(&mut self, tx: &Transaction) -> Result<(), ChainError> {
        let Some(addr_s) = tx.parse_slash_settle_memo() else {
            return Ok(());
        };
        let Some(addr) = Address::from_hex(&addr_s) else {
            return Ok(());
        };
        let key = addr.to_hex();
        let Some(mut rec) = self.data.node_bonds.get(&key).cloned() else {
            // Peer without local bond meta still applied UTXOs; nothing to clear.
            return Ok(());
        };
        let spent: std::collections::HashSet<(String, u32)> = tx
            .inputs
            .iter()
            .map(|i| (i.prev_txid.to_hex(), i.vout))
            .collect();
        rec.locked
            .retain(|l| !spent.contains(&(l.txid_hex.clone(), l.vout)));
        rec.slashed = true;
        if rec.slashed_to_vault_atomic == 0 {
            rec.slashed_to_vault_atomic = tx.total_output().atomic();
        }
        rec.stake_atomic = rec.locked_atomic();
        rec.unlock_after_height = 0;
        self.data.node_scores.remove(&key);
        self.data.node_bonds.insert(key, rec);
        self.clear_preferred_slash_settle(&addr);
        self.touch();
        Ok(())
    }

    /// Auto-bond when stake is present (used by local relay credits).
    pub fn ensure_node_bond(
        &mut self,
        address: &Address,
        peer_id: &str,
        at_height: u64,
    ) -> Result<Option<NodeBondRec>, ChainError> {
        if self.is_node_bond_eligible(address) {
            return Ok(self.bond_for(address).cloned());
        }
        if self.spendable_balance(address).atomic() < MIN_NODE_BOND_ATOMIC
            && self.balance(address).atomic() < MIN_NODE_BOND_ATOMIC
        {
            return Ok(None);
        }
        match self.register_node_bond(address, peer_id, at_height) {
            Ok(r) => Ok(Some(r)),
            Err(_) => Ok(None),
        }
    }

    pub fn has_ai_receipt(&self, job_id: &str) -> bool {
        self.data.ai_receipts.iter().any(|r| r.job_id == job_id)
    }

    /// Insert receipt if `job_id` is new. Returns `true` when stored.
    pub fn push_ai_receipt(&mut self, receipt: mesh_types::AiJobReceipt) -> Result<bool, ChainError> {
        if self.has_ai_receipt(&receipt.job_id) {
            return Ok(false);
        }
        if matches!(receipt.job_kind, mesh_types::AiJobKind::ProtocolEval)
            && !receipt.research_scenario.is_empty()
        {
            let cert = mesh_types::ImprovementCertificate {
                scenario: receipt.research_scenario.clone(),
                primary: receipt.score_primary,
                height: self.height(),
                job_id: receipt.job_id.clone(),
                verified_at: receipt.verified_at,
            };
            self.data.improvement_certs.push(cert);
            const CERT_CAP: usize = 128;
            if self.data.improvement_certs.len() > CERT_CAP {
                let n = self.data.improvement_certs.len() - CERT_CAP;
                self.data.improvement_certs.drain(0..n);
            }
        }
        const CAP: usize = 500;
        self.data.ai_receipts.push(receipt);
        if self.data.ai_receipts.len() > CAP {
            let n = self.data.ai_receipts.len() - CAP;
            self.data.ai_receipts.drain(0..n);
        }
        // Batch with score dirty flush (N2) — avoid full chain.bin rewrite per receipt.
        self.touch();
        Ok(true)
    }

    /// Clear pending market scores after they were paid in a coinbase.
    pub fn clear_market_scores(&mut self) -> Result<(), ChainError> {
        self.data.gpu_scores.clear();
        self.data.node_scores.clear();
        self.scores_epoch = self.scores_epoch.saturating_add(1);
        self.persist()
    }

    pub fn proposals(&self) -> &[mesh_types::ParamProposal] {
        &self.data.proposals
    }

    pub fn active_envelopes(&self) -> &mesh_types::ProtocolEnvelopes {
        &self.data.active_envelopes
    }

    pub fn last_auto_adapt_at_height(&self) -> u64 {
        self.data.last_auto_adapt_at_height
    }

    pub fn last_auto_adapt_proposal_id(&self) -> &str {
        &self.data.last_auto_adapt_proposal_id
    }

    pub fn last_auto_adapt_eval_count(&self) -> u64 {
        self.data.last_auto_adapt_eval_count
    }

    pub fn last_retarget_adapt_grover_count(&self) -> u64 {
        self.data.last_retarget_adapt_grover_count
    }

    pub fn set_last_retarget_adapt_grover_count(&mut self, n: u64) {
        self.data.last_retarget_adapt_grover_count = n;
    }

    pub fn improvement_certs(&self) -> &[mesh_types::ImprovementCertificate] {
        &self.data.improvement_certs
    }

    pub fn scenario_eval_count(&self, scenario: &str) -> u64 {
        self.data
            .ai_receipts
            .iter()
            .filter(|r| {
                matches!(r.job_kind, mesh_types::AiJobKind::ProtocolEval)
                    && r.research_scenario == scenario
            })
            .count() as u64
    }

    pub fn grover_eval_count(&self) -> u64 {
        self.scenario_eval_count("quantum_grover")
    }

    pub fn grover_certs_since_retarget_adapt(&self) -> u64 {
        self.grover_eval_count()
            .saturating_sub(self.data.last_retarget_adapt_grover_count)
    }

    pub fn param_epoch(&self) -> u64 {
        self.data.param_epoch
    }

    pub fn epoch_history(&self) -> &[mesh_types::ParamEpoch] {
        &self.data.epoch_history
    }

    pub fn protocol_eval_receipt_count(&self) -> u64 {
        self.data
            .ai_receipts
            .iter()
            .filter(|r| matches!(r.job_kind, mesh_types::AiJobKind::ProtocolEval))
            .count() as u64
    }

    /// Apply soft envelopes from a proposal and record auto-adapt metadata.
    pub fn apply_soft_auto_adapt(
        &mut self,
        mut proposal: mesh_types::ParamProposal,
        at_height: u64,
        eval_count: u64,
    ) -> Result<mesh_types::ParamProposal, ChainError> {
        self.data.next_proposal_id = self.data.next_proposal_id.saturating_add(1);
        proposal.id = format!("prop-{}", self.data.next_proposal_id);
        proposal.status = mesh_types::ProposalStatus::Activated;
        if !proposal.rationale.contains("auto-adapt") {
            proposal.rationale = format!("auto-adapt: {}", proposal.rationale);
        }
        let prev = self.data.active_envelopes.clone();
        let mut env = proposal.envelopes.clone().clamp();
        env.freeze_retarget_from(&prev);
        env.limit_retarget_jump(&prev);
        if env.retarget_changed(&prev) {
            self.data.last_retarget_adapt_grover_count = self.grover_eval_count();
            if !proposal.rationale.contains("quantum-gated-retarget") {
                proposal.rationale = format!(
                    "{}; quantum-gated-retarget grover_n={}",
                    proposal.rationale,
                    self.data.last_retarget_adapt_grover_count
                );
            }
        }
        proposal.envelopes = env.clone();
        self.data.active_envelopes = env;
        self.data.last_auto_adapt_at_height = at_height;
        self.data.last_auto_adapt_proposal_id = proposal.id.clone();
        self.data.last_auto_adapt_eval_count = eval_count;
        self.data.param_epoch = self.data.param_epoch.saturating_add(1);
        let epoch = mesh_types::ParamEpoch {
            epoch: self.data.param_epoch,
            height: at_height,
            proposal_id: proposal.id.clone(),
            rationale: proposal.rationale.clone(),
            eval_count,
            envelopes: self.data.active_envelopes.clone(),
        };
        self.data.epoch_history.push(epoch);
        const EPOCH_CAP: usize = 50;
        if self.data.epoch_history.len() > EPOCH_CAP {
            let n = self.data.epoch_history.len() - EPOCH_CAP;
            self.data.epoch_history.drain(0..n);
        }
        self.data.proposals.push(proposal.clone());
        const CAP: usize = 100;
        if self.data.proposals.len() > CAP {
            let n = self.data.proposals.len() - CAP;
            self.data.proposals.drain(0..n);
        }
        // Batch to disk via research_tick flush — fsync under chain lock stalled RPC for seconds.
        self.touch();
        Ok(proposal)
    }

    pub fn push_proposal(
        &mut self,
        mut proposal: mesh_types::ParamProposal,
    ) -> Result<mesh_types::ParamProposal, ChainError> {
        self.data.next_proposal_id = self.data.next_proposal_id.saturating_add(1);
        proposal.id = format!("prop-{}", self.data.next_proposal_id);
        self.data.proposals.push(proposal.clone());
        const CAP: usize = 100;
        if self.data.proposals.len() > CAP {
            let n = self.data.proposals.len() - CAP;
            self.data.proposals.drain(0..n);
        }
        self.persist()?;
        Ok(proposal)
    }

    pub fn activate_proposal(&mut self, id: &str) -> Result<mesh_types::ProtocolEnvelopes, ChainError> {
        let Some(p) = self.data.proposals.iter_mut().find(|p| p.id == id) else {
            return Err(ChainError::InvalidTx(format!("unknown proposal {id}")));
        };
        if !matches!(p.status, mesh_types::ProposalStatus::Pending) {
            return Err(ChainError::InvalidTx("proposal not pending".into()));
        }
        p.status = mesh_types::ProposalStatus::Activated;
        self.data.active_envelopes = p.envelopes.clone().clamp();
        // BPS suggestions are recorded but not applied to emission (needs governance vote).
        self.persist()?;
        Ok(self.data.active_envelopes.clone())
    }

    pub fn reject_proposal(&mut self, id: &str) -> Result<(), ChainError> {
        let Some(p) = self.data.proposals.iter_mut().find(|p| p.id == id) else {
            return Err(ChainError::InvalidTx(format!("unknown proposal {id}")));
        };
        if !matches!(p.status, mesh_types::ProposalStatus::Pending) {
            return Err(ChainError::InvalidTx("proposal not pending".into()));
        }
        p.status = mesh_types::ProposalStatus::Rejected;
        self.persist()
    }

    /// Cast one soft-envelope vote per `node_id`. Duplicate votes are rejected.
    /// Majority yes → activate envelopes; majority no → reject.
    pub fn cast_proposal_vote(
        &mut self,
        id: &str,
        node_id: &str,
        choice: mesh_types::VoteChoice,
        at_height: u64,
    ) -> Result<mesh_types::ParamProposal, ChainError> {
        let node_id = node_id.trim();
        if node_id.is_empty() {
            return Err(ChainError::InvalidTx("node_id required".into()));
        }
        let Some(p) = self.data.proposals.iter_mut().find(|p| p.id == id) else {
            return Err(ChainError::InvalidTx(format!("unknown proposal {id}")));
        };
        if !matches!(p.status, mesh_types::ProposalStatus::Pending) {
            if p.votes.iter().any(|v| v.node_id == node_id) {
                return Err(ChainError::InvalidTx(
                    "this node_id already voted on this proposal".into(),
                ));
            }
            return Err(ChainError::InvalidTx("proposal not pending".into()));
        }
        if p.votes.iter().any(|v| v.node_id == node_id) {
            return Err(ChainError::InvalidTx(
                "this node_id already voted on this proposal".into(),
            ));
        }
        p.votes.push(mesh_types::ProposalVote {
            node_id: node_id.to_string(),
            choice,
            at_height,
        });
        let (yes, no) = p.vote_counts();
        if yes > no {
            p.status = mesh_types::ProposalStatus::Activated;
            self.data.active_envelopes = p.envelopes.clone().clamp();
        } else if no > yes {
            p.status = mesh_types::ProposalStatus::Rejected;
        }
        let out = p.clone();
        self.persist()?;
        Ok(out)
    }

    pub fn set_active_envelopes(
        &mut self,
        env: mesh_types::ProtocolEnvelopes,
    ) -> Result<(), ChainError> {
        self.data.active_envelopes = env.clamp();
        self.persist()
    }
}

pub fn apply_block_utxos(
    utxos: &mut HashMap<OutPoint, Utxo>,
    block: &Block,
) -> Result<(), ChainError> {
    for tx in &block.txs {
        apply_tx_utxos(utxos, tx, /*allow_missing*/ false)?;
    }
    Ok(())
}

fn migrate_monolithic(path: &Path, data: &ChainData) -> Result<(), ChainError> {
    let wal = ChainStore::wal_path(path);
    let meta = ChainStore::meta_path(path);
    write_blocks_wal(&wal, &data.blocks)?;
    let bytes = bincode::serialize(&data.to_meta()).map_err(|e| ChainError::Store(e.to_string()))?;
    let tmp = meta.with_extension("meta.bin.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, &meta)?;
    let bak = path.with_extension("bin.monolithic-bak");
    let _ = fs::rename(path, &bak);
    eprintln!(
        "mesh-chain: migrated monolithic {} → {}.blocks.wal + {}.meta.bin (bak {})",
        path.display(),
        path.display(),
        path.display(),
        bak.display()
    );
    Ok(())
}

fn utxo_ckpt_to_map(ckpt: &UtxoCheckpoint) -> Result<HashMap<OutPoint, Utxo>, ChainError> {
    let mut map = HashMap::with_capacity(ckpt.utxos.len());
    for (txid_hex, vout, addr_s, atomic) in &ckpt.utxos {
        let txid = Hash::from_hex(txid_hex)
            .map_err(|e| ChainError::Store(format!("ckpt txid: {e}")))?;
        let address = Address::from_hex(addr_s)
            .ok_or_else(|| ChainError::Store(format!("ckpt bad address {addr_s}")))?;
        map.insert(
            OutPoint::new(txid, *vout),
            Utxo {
                address,
                amount: Amount::from_atomic(*atomic),
            },
        );
    }
    Ok(map)
}

fn write_blocks_wal(path: &Path, blocks: &[Block]) -> Result<(), ChainError> {
    use std::io::Write;
    let tmp = path.with_extension("blocks.wal.tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(WAL_MAGIC)?;
    for b in blocks {
        let bytes = bincode::serialize(b).map_err(|e| ChainError::Store(e.to_string()))?;
        let len = u32::try_from(bytes.len()).map_err(|_| ChainError::Store("block too large".into()))?;
        f.write_all(&len.to_le_bytes())?;
        f.write_all(&bytes)?;
    }
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)?;
    Ok(())
}

fn wal_fsync_enabled() -> bool {
    // Default ON for seed durability; set MESH_WAL_FSYNC=0 to relax under extreme write load.
    std::env::var("MESH_WAL_FSYNC")
        .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
        .unwrap_or(true)
}

fn append_block_wal(path: &Path, block: &Block) -> Result<(), ChainError> {
    use std::io::{Seek, SeekFrom, Write};
    let bytes = bincode::serialize(block).map_err(|e| ChainError::Store(e.to_string()))?;
    let len = u32::try_from(bytes.len()).map_err(|_| ChainError::Store("block too large".into()))?;
    if !path.exists() {
        write_blocks_wal(path, std::slice::from_ref(block))?;
        return Ok(());
    }
    let mut f = fs::OpenOptions::new().append(true).open(path)?;
    // Ensure magic present (empty/corrupt → rewrite).
    if f.metadata()?.len() == 0 {
        drop(f);
        write_blocks_wal(path, std::slice::from_ref(block))?;
        return Ok(());
    }
    f.write_all(&len.to_le_bytes())?;
    f.write_all(&bytes)?;
    if wal_fsync_enabled() {
        f.sync_all()?;
    } else {
        f.flush()?;
    }
    let _ = f.seek(SeekFrom::End(0));
    Ok(())
}

fn read_blocks_wal(path: &Path) -> Result<Vec<Block>, ChainError> {
    use std::io::Read;
    let mut f = fs::File::open(path)?;
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic)
        .map_err(|e| ChainError::Store(format!("wal magic: {e}")))?;
    if &magic != WAL_MAGIC {
        return Err(ChainError::Store("bad blocks.wal magic".into()));
    }
    let mut blocks = Vec::new();
    loop {
        let mut len_buf = [0u8; 4];
        match f.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(ChainError::Store(e.to_string())),
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        if len == 0 || len > 64 * 1024 * 1024 {
            return Err(ChainError::Store(format!("wal block len {len} invalid")));
        }
        let mut buf = vec![0u8; len];
        f.read_exact(&mut buf)
            .map_err(|e| ChainError::Store(format!("wal block body: {e}")))?;
        let block: Block =
            bincode::deserialize(&buf).map_err(|e| ChainError::Store(format!("wal block: {e}")))?;
        blocks.push(block);
    }
    Ok(blocks)
}

fn node_bond_required() -> bool {
    match std::env::var("MESH_NODE_BOND") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !(v == "0" || v == "false" || v == "off")
        }
        // Default on — empty wallets cannot farm node-market Sybil credits.
        Err(_) => true,
    }
}

pub fn apply_tx_utxos(
    utxos: &mut HashMap<OutPoint, Utxo>,
    tx: &Transaction,
    allow_missing_for_reindex: bool,
) -> Result<(), ChainError> {
    let txid = tx.txid();

    if !tx.is_coinbase() {
        for inp in &tx.inputs {
            let op = OutPoint::new(inp.prev_txid, inp.vout);
            if utxos.remove(&op).is_none() && !allow_missing_for_reindex {
                return Err(ChainError::InvalidTx(format!("missing utxo {op}")));
            }
        }
    }

    for (vout, out) in tx.outputs.iter().enumerate() {
        let op = OutPoint::new(txid, vout as u32);
        utxos.insert(
            op,
            Utxo {
                address: out.address,
                amount: out.amount,
            },
        );
    }
    Ok(())
}
