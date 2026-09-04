//! Local chain engine: genesis, validation, mining templates, file persistence.

mod difficulty;
mod emission;
mod finality;
mod meta_migrate;
mod store;
mod transfer;
mod validate;

#[cfg(test)]
mod pomc_tests;
#[cfg(test)]
mod gov_tests;
#[cfg(test)]
mod bond_tests;
#[cfg(test)]
mod validate_tests;
#[cfg(test)]
mod fork_tests;
#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod finality_tests;

pub use difficulty::{
    next_difficulty, next_difficulty_from_window, next_difficulty_from_window_with,
    next_difficulty_with, RetargetParams, INITIAL_DIFFICULTY, MAX_DIFFICULTY, MIN_DIFFICULTY,
    RETARGET_INTERVAL,
};
pub use emission::{
    block_reward, cpu_market_reward, cpu_market_reward_with, emitted_before_atomic,
    gpu_market_reward, gpu_market_reward_with, gpu_scores_with_fusion_credit, node_market_reward,
    supply_cap_atomic, ERA_BLOCKS,
};
pub use finality::{
    finality_activation_height, finality_active_at, finality_bond_age, finality_min_attestors,
    finality_window, min_finality_bond_atomic, FinalityAttestation, FinalityIngest, FinalityState,
    DEFAULT_FINALITY_HEIGHT, DEFAULT_FINALITY_MIN_BOND_ATOMIC, DEFAULT_FINALITY_WINDOW,
};
pub use store::{
    ChainStore, ColdPrunePlan, LockedBondUtxo, NodeBondRec, TipSnapshot,
    BOND_UNLOCK_COOLDOWN_BLOCKS, MAX_CREDIT_PER_EVENT, MAX_PENDING_GPU_WEIGHT,
    MAX_PENDING_NODE_WEIGHT, MIN_COLD_PRUNE_KEEP, MIN_NODE_BOND_ATOMIC,
};
pub use transfer::{build_signed_payment, build_slash_settle};
pub use mesh_types::COINBASE_MATURITY;
pub use validate::{
    validate_block, validate_block_ex, validate_mempool_tx, validate_tx, MAX_BLOCK_TXS,
    MAX_MEMO_BYTES,
};

use mesh_crypto::Keypair;
use mesh_types::{
    Address, Amount, Block, BlockHeader, Hash, OutPoint, Transaction, Utxo,
    TARGET_BLOCK_TIME_SECS,
};
use meshhash_cpu::{meshhash_cpu_with_params, MeshHashParams};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Default difficulty for local/dev networks (leading zero bits).
/// Prefer [`Chain::next_difficulty`]; this alias keeps older CLI defaults working.
pub const DEV_DIFFICULTY: u32 = INITIAL_DIFFICULTY;

/// Genesis message embedded in the genesis coinbase.
pub const GENESIS_MEMO: &str = "MonkeyMesh genesis — adaptive compute network";

/// Light MeshHash is **test-only**. Production / testnet binaries always rematch
/// full Fusion. `MESH_LIGHT_POW` is ignored outside `cfg(test)` so a mis-set
/// env cannot mint easy blocks.
fn light_pow_from_env() -> bool {
    cfg!(test)
}

#[derive(Debug, Error)]
pub enum ChainError {
    #[error("block validation failed: {0}")]
    InvalidBlock(String),
    #[error("invalid transaction: {0}")]
    InvalidTx(String),
    #[error("insufficient funds: have {have}, need {need}")]
    InsufficientFunds { have: Amount, need: Amount },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("store error: {0}")]
    Store(String),
}

pub struct Chain {
    store: ChainStore,
    /// Use light MeshHash for faster local mining (false = full v1/v2 by height).
    pub light_pow: bool,
    /// Local node operator address — receives node-market credit for relay work.
    pub node_operator: Option<Address>,
    /// Soft dampener from local mesh median peer RTT (1000 = full). Build/27 B8.
    pub relay_rtt_factor_milli: u64,
    /// Orphan / competing tips awaiting connection or depth-1 replace (capped).
    orphans: std::collections::HashMap<Hash, Block>,
    /// Economic finality (Build/36 F2). Default off (`MESH_FINALITY_HEIGHT`).
    finality: FinalityState,
}

const MAX_ORPHANS: usize = 64;

/// Consensus PoW hash for a header (MeshHash v1/v2 or Evo v3 recycle seed).
pub fn pow_hash_header(
    commitment: &Hash,
    nonce: u64,
    light: bool,
    height: u64,
    prev_hash: &Hash,
) -> Hash {
    let (seed, params) = meshhash_cpu::pow_search_inputs(commitment, light, height, prev_hash);
    meshhash_cpu_with_params(&seed, nonce, &params)
}

/// PoW work key: more leading zeros wins; tie-break on block id bytes.
pub fn block_pow_work(block: &Block, light_pow: bool) -> (u32, [u8; 32]) {
    let commitment = block.header.pre_pow_commitment();
    let pow = pow_hash_header(
        &commitment,
        block.header.nonce,
        light_pow,
        block.header.height,
        &block.header.prev_hash,
    );
    let mut id = [0u8; 32];
    id.copy_from_slice(block.id().as_bytes());
    (pow.leading_zero_bits(), id)
}

pub(crate) fn work_strictly_better(a: (u32, [u8; 32]), b: (u32, [u8; 32])) -> bool {
    a.0 > b.0 || (a.0 == b.0 && a.1 > b.1)
}

/// Soft RTT performance factor for node-market credits (Build/27 B8).
/// `None` (no samples) → neutral 1000; ≤50ms → 1000; ≤200ms → 850; else → 700.
pub fn rtt_factor_milli(median_rtt_ms: Option<u64>) -> u64 {
    match median_rtt_ms {
        None => 1_000,
        Some(ms) if ms <= 50 => 1_000,
        Some(ms) if ms <= 200 => 850,
        Some(_) => 700,
    }
}

impl Chain {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ChainError> {
        let store = ChainStore::open(path)?;
        let mut finality = FinalityState::load(store.path(), store.genesis_hash());
        if finality.finalized_height > 0 {
            let matches = store
                .get_by_height(finality.finalized_height)
                .map(|b| b.id() == finality.finalized_hash)
                .unwrap_or(false);
            if !matches {
                tracing::warn!(
                    height = finality.finalized_height,
                    "finality sidecar does not match local chain — ignoring"
                );
                finality = FinalityState::default();
            }
        }
        Ok(Self {
            store,
            light_pow: light_pow_from_env(),
            node_operator: None,
            relay_rtt_factor_milli: 1_000,
            orphans: std::collections::HashMap::new(),
            finality,
        })
    }

    /// Remove on-disk chain files (height-0 private genesis, corrupt replica).
    pub fn wipe_store_files(path: impl AsRef<Path>) {
        ChainStore::wipe_files(path);
    }

    pub fn open_or_genesis(path: impl AsRef<Path>) -> Result<Self, ChainError> {
        let mut chain = Self::open(path)?;
        if chain.store.is_empty() {
            let genesis = build_genesis(chain.light_pow)?;
            validate_block(
                &genesis,
                None,
                chain.light_pow,
                &Default::default(),
                None,
                &Default::default(),
            )?;
            chain.store.append(&genesis)?;
            tracing::info!(
                height = 0,
                id = %genesis.id(),
                light_pow = chain.light_pow,
                "genesis created"
            );
        }
        Ok(chain)
    }

    /// Recent coinbase outpoints still within the maturity window.
    pub fn recent_coinbase_heights(&self) -> std::collections::HashMap<OutPoint, u64> {
        use std::collections::HashMap;
        let mut map = HashMap::new();
        if self.store.is_empty() {
            return map;
        }
        let tip = self.height();
        let start = tip.saturating_sub(COINBASE_MATURITY.saturating_sub(1));
        for h in start..=tip {
            if let Some(b) = self.get_block(h) {
                if let Some(cb) = b.txs.first() {
                    let txid = cb.txid();
                    for vout in 0..cb.outputs.len() {
                        map.insert(OutPoint::new(txid, vout as u32), h);
                    }
                }
            }
        }
        map
    }

    pub fn genesis_hash(&self) -> Hash {
        self.store.genesis_hash()
    }

    pub fn apply_cold_prune(&mut self, keep_blocks: u64) -> Result<ColdPrunePlan, ChainError> {
        self.store.apply_cold_prune(keep_blocks)
    }

    /// Soft slash mark from P2P (multi-seed freeze before settle confirms).
    pub fn apply_slash_mark(
        &mut self,
        address: mesh_types::Address,
        height: u64,
        stake_atomic: u64,
        peer_id: &str,
        preferred_settle_txid: &str,
    ) -> Result<NodeBondRec, ChainError> {
        self.store.apply_slash_mark(
            &address,
            height,
            stake_atomic,
            peer_id,
            preferred_settle_txid,
        )
    }

    /// Preferred settle txid from SlashMark (if any).
    pub fn preferred_slash_settle(&self, address: &Address) -> Option<String> {
        self.store
            .preferred_slash_settle(address)
            .map(|s| s.to_string())
    }

    /// Fingerprint of mempool contents for template-cache invalidation.
    pub fn mempool_fingerprint(&self) -> u64 {
        let mut h = self.store.mempool().len() as u64;
        for tx in self.store.mempool() {
            let id = tx.txid();
            let b = id.as_bytes();
            let chunk = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
            h = h
                .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                .wrapping_add(chunk);
        }
        h
    }

    pub fn scores_epoch(&self) -> u64 {
        self.store.scores_epoch()
    }

    pub fn blocks_from(&self, from_height: u64, limit: u32) -> Vec<Block> {
        let mut out = Vec::new();
        let tip_h = self.height();
        if self.store.is_empty() {
            return out;
        }
        let mut h = from_height;
        while h <= tip_h && out.len() < limit as usize {
            if let Some(b) = self.get_block(h) {
                out.push(b);
            }
            h += 1;
        }
        out
    }

    /// Validate and append a block that extends the tip, or reorg to a better-work
    /// fork (depth-1 same-parent, or up to coinbase-maturity via orphans).
    pub fn import_block(&mut self, block: Block) -> Result<bool, ChainError> {
        let id = block.id();
        if let Some(tip) = self.store.tip() {
            if id == tip.id() {
                return Ok(false);
            }
            // Competing tip at same height sharing the same parent → depth-1 fork choice.
            if block.header.height == tip.header.height
                && block.header.prev_hash == tip.header.prev_hash
            {
                let challenger_work = block_pow_work(&block, self.light_pow);
                let tip_work = block_pow_work(&tip, self.light_pow);
                if !work_strictly_better(challenger_work, tip_work) {
                    self.remember_orphan(block);
                    return Ok(false);
                }
                let parent = self
                    .store
                    .get_by_height(tip.header.height.saturating_sub(1))
                    .ok_or_else(|| ChainError::InvalidBlock("missing parent for reorg".into()))?;
                if parent.id() != block.header.prev_hash {
                    return Err(ChainError::InvalidBlock("reorg parent mismatch".into()));
                }
                if self
                    .finality
                    .would_pop_finalized(tip.header.height.saturating_sub(1))
                {
                    return Err(ChainError::InvalidBlock(
                        "cannot reorg a finalized height".into(),
                    ));
                }
                let expected_diff = tip.header.difficulty;
                let old = self.pop_tip_checked()?;
                self.remember_orphan(old);
                self.append_validated(block, Some(&parent), Some(expected_diff), false)?;
                tracing::info!(
                    height = parent.header.height.saturating_add(1),
                    id = %id,
                    "depth-1 reorg: adopted better-work tip"
                );
                let _ = self.try_connect_orphans();
                return Ok(true);
            }

            if block.header.height != tip.header.height + 1 || block.header.prev_hash != tip.id() {
                self.remember_orphan(block.clone());
                if self.try_reorg_to(&block)? {
                    let _ = self.try_connect_orphans();
                    return Ok(true);
                }
                return Ok(false);
            }
            let expected_diff = self.next_difficulty();
            self.append_validated(block, Some(&tip), Some(expected_diff), false)?;
        } else {
            if block.header.height != 0 {
                return Err(ChainError::InvalidBlock(
                    "first block must be genesis".into(),
                ));
            }
            self.append_validated(block, None, None, false)?;
        }
        let _ = self.try_connect_orphans();
        Ok(true)
    }

    /// Import a block from the official seed HTTP snapshot.
    ///
    /// Header difficulty is trusted (live testnet retarget 20→15). Fusion PoW is
    /// skipped for historical IBD (`verify_pow = false`); the tail is fully hashed.
    pub fn import_official_snapshot_block(
        &mut self,
        block: Block,
        verify_pow: bool,
    ) -> Result<bool, ChainError> {
        let header_diff = block.header.difficulty;
        let id = block.id();
        if let Some(tip) = self.store.tip() {
            if id == tip.id() {
                return Ok(false);
            }
            if block.header.height != tip.header.height + 1 || block.header.prev_hash != tip.id() {
                return Ok(false);
            }
            self.append_validated(block, Some(&tip), Some(header_diff), !verify_pow)?;
        } else {
            if block.header.height != 0 {
                return Err(ChainError::InvalidBlock(
                    "first block must be genesis".into(),
                ));
            }
            self.append_validated(block, None, None, !verify_pow)?;
        }
        Ok(true)
    }

    pub fn set_bulk_import(&mut self, bulk: bool) {
        self.store.set_bulk_import(bulk);
    }

    /// Load official UTXO snapshot + hot block tail (skips replaying 28k Fusion blocks).
    pub fn install_official_prune(
        &mut self,
        genesis: Hash,
        utxos: std::collections::HashMap<OutPoint, Utxo>,
        hot_blocks: Vec<Block>,
    ) -> Result<(), ChainError> {
        self.store
            .install_official_prune(genesis, utxos, hot_blocks)
    }

    fn append_validated(
        &mut self,
        block: Block,
        prev: Option<&Block>,
        expected_diff: Option<u32>,
        skip_pow: bool,
    ) -> Result<(), ChainError> {
        let cbs = self.recent_coinbase_heights();
        validate_block_ex(
            &block,
            prev,
            self.light_pow,
            self.store.utxos(),
            expected_diff,
            &cbs,
            skip_pow,
        )?;
        self.reject_locked_spends(&block)?;
        self.store.append(&block)?;
        self.finalize_slash_settles(&block)?;
        let _ = self.store.clear_market_scores();
        self.maybe_advance_finality();
        Ok(())
    }

    /// Walk `new_tip` back through orphans to a common ancestor. Adopt the fork
    /// when it has strictly more PoW and the reorg is shallower than coinbase maturity.
    fn try_reorg_to(&mut self, new_tip: &Block) -> Result<bool, ChainError> {
        const MAX_REORG: u64 = mesh_types::COINBASE_MATURITY;
        let Some(tip) = self.store.tip() else {
            return Ok(false);
        };
        if new_tip.id() == tip.id() {
            return Ok(false);
        }

        let mut fork: Vec<Block> = vec![new_tip.clone()];
        let mut cursor = new_tip.header.prev_hash;
        for _ in 0..MAX_REORG {
            if cursor == tip.id() {
                return Ok(false);
            }
            if let Some(on_disk) = self.store.get_by_hash(&cursor) {
                let ancestor_h = on_disk.header.height;
                let depth = tip.header.height.saturating_sub(ancestor_h);
                if depth == 0 || depth > MAX_REORG {
                    return Ok(false);
                }
                let mut ours = Vec::new();
                for h in (ancestor_h + 1)..=tip.header.height {
                    let Some(b) = self.store.get_by_height(h) else {
                        return Ok(false);
                    };
                    ours.push(b);
                }
                if !self.fork_has_more_work(&fork, &ours) {
                    return Ok(false);
                }
                if self.finality.would_pop_finalized(ancestor_h) {
                    return Err(ChainError::InvalidBlock(
                        "cannot reorg a finalized height".into(),
                    ));
                }
                for _ in 0..depth {
                    let old = self.pop_tip_checked()?;
                    self.remember_orphan(old);
                }
                let mut apply = fork;
                apply.reverse();
                for b in apply {
                    self.orphans.remove(&b.id());
                    let parent = self.store.tip();
                    let expected = parent.as_ref().map(|_| self.next_difficulty());
                    if let Err(e) = self.append_validated(b, parent.as_ref(), expected, false) {
                        for old in ours {
                            let p = self.store.tip();
                            let exp = p.as_ref().map(|_| self.next_difficulty());
                            let _ = self.append_validated(old, p.as_ref(), exp, false);
                        }
                        return Err(e);
                    }
                }
                tracing::info!(
                    height = new_tip.header.height,
                    id = %new_tip.id(),
                    depth,
                    "reorg: adopted better-work fork"
                );
                return Ok(true);
            }
            if let Some(prev) = self.orphans.get(&cursor).cloned() {
                cursor = prev.header.prev_hash;
                fork.push(prev);
                continue;
            }
            return Ok(false);
        }
        Ok(false)
    }

    fn fork_has_more_work(&self, theirs_newest_first: &[Block], ours: &[Block]) -> bool {
        let t: u128 = theirs_newest_first
            .iter()
            .map(|b| u128::from(block_pow_work(b, self.light_pow).0))
            .sum();
        let o: u128 = ours
            .iter()
            .map(|b| u128::from(block_pow_work(b, self.light_pow).0))
            .sum();
        if t != o {
            return t > o;
        }
        let t_id = theirs_newest_first
            .first()
            .map(|b| b.id().0)
            .unwrap_or([0u8; 32]);
        let o_id = ours.last().map(|b| b.id().0).unwrap_or([0u8; 32]);
        t_id > o_id
    }

    fn remember_orphan(&mut self, block: Block) {
        if self.orphans.len() >= MAX_ORPHANS {
            // Drop an arbitrary entry to bound memory.
            if let Some(k) = self.orphans.keys().next().cloned() {
                self.orphans.remove(&k);
            }
        }
        self.orphans.insert(block.id(), block);
    }

    /// Try to extend tip from orphan buffer (depth-1 connect only).
    fn try_connect_orphans(&mut self) -> Result<(), ChainError> {
        let tip = match self.store.tip() {
            Some(t) => t,
            None => return Ok(()),
        };
        let tip_id = tip.id();
        let next_h = tip.header.height + 1;
        let candidates: Vec<Block> = self
            .orphans
            .values()
            .filter(|b| b.header.height == next_h && b.header.prev_hash == tip_id)
            .cloned()
            .collect();
        // Prefer best work among candidates.
        let mut best: Option<Block> = None;
        let mut best_work = (0u32, [0u8; 32]);
        for b in candidates {
            let w = block_pow_work(&b, self.light_pow);
            if best.is_none() || work_strictly_better(w, best_work) {
                best_work = w;
                best = Some(b);
            }
        }
        if let Some(b) = best {
            self.orphans.remove(&b.id());
            let _ = self.import_block(b)?;
        }
        Ok(())
    }

    fn reject_locked_spends(&self, block: &Block) -> Result<(), ChainError> {
        for tx in &block.txs {
            if tx.is_coinbase() {
                continue;
            }
            if self.tx_spends_locked(tx) && !self.is_valid_slash_settle(tx) {
                return Err(ChainError::InvalidBlock(
                    "tx spends bonded UTXO (not a valid slash settle)".into(),
                ));
            }
        }
        Ok(())
    }

    fn tx_spends_locked(&self, tx: &Transaction) -> bool {
        tx.inputs.iter().any(|inp| {
            self.store
                .is_outpoint_locked(&OutPoint::new(inp.prev_txid, inp.vout))
        })
    }

    /// Valid on-chain slash settle: memo + single vault output + inputs owned by memo addr.
    pub fn is_valid_slash_settle(&self, tx: &Transaction) -> bool {
        let Some(addr_s) = tx.parse_slash_settle_memo() else {
            return false;
        };
        let Some(addr) = Address::from_hex(&addr_s) else {
            return false;
        };
        let vault = deferred_slash_vault();
        if tx.outputs.len() != 1 || tx.outputs[0].address != vault {
            return false;
        }
        if tx.outputs[0].amount == Amount::ZERO {
            return false;
        }
        let mut sum = 0u64;
        for inp in &tx.inputs {
            let op = OutPoint::new(inp.prev_txid, inp.vout);
            let Some(u) = self.store.utxos().get(&op) else {
                return false;
            };
            if u.address != addr {
                return false;
            }
            sum = sum.saturating_add(u.amount.atomic());
            // Local bond meta: inputs must be from the locked set. Peers without
            // bond records skip this and still accept via normal UTXO rules.
            if let Some(bond) = self.store.bond_for(&addr) {
                let locked = bond
                    .locked
                    .iter()
                    .any(|l| l.txid_hex == op.txid.to_hex() && l.vout == op.vout);
                if !locked {
                    return false;
                }
            }
        }
        sum == tx.outputs[0].amount.atomic()
    }

    fn finalize_slash_settles(&mut self, block: &Block) -> Result<(), ChainError> {
        for tx in &block.txs {
            if tx.is_slash_settle() {
                self.store.apply_slash_settle_tx(tx)?;
            }
        }
        Ok(())
    }

    /// Difficulty required for the block that extends the current tip.
    pub fn next_difficulty(&self) -> u32 {
        let params = difficulty::RetargetParams::from_envelopes(self.store.active_envelopes());
        let Some(tip) = self.tip() else {
            return 1;
        };
        if tip.header.height == 0 {
            return difficulty::INITIAL_DIFFICULTY.max(params.min_diff());
        }
        let next_height = tip.header.height + 1;
        let interval = params.interval.max(1);
        if next_height % interval != 0 {
            return tip
                .header
                .difficulty
                .clamp(params.min_diff(), difficulty::MAX_DIFFICULTY);
        }
        let start_height = next_height.saturating_sub(interval);
        let Some(start) = self.get_block(start_height) else {
            return tip.header.difficulty.max(params.min_diff());
        };
        difficulty::next_difficulty_from_window_with(&tip, &start, params)
    }

    /// Live retarget knobs from active envelopes (Build/30).
    pub fn retarget_params(&self) -> difficulty::RetargetParams {
        difficulty::RetargetParams::from_envelopes(self.store.active_envelopes())
    }

    /// Soft mining hint = consensus difficulty ± activated bias.
    /// Informational / local UX only — does **not** change consensus validation.
    pub fn soft_mining_diff_hint(&self) -> u32 {
        let base = self.next_difficulty() as i64;
        let bias = self.store.active_envelopes().suggested_cpu_diff_bias as i64;
        (base + bias).clamp(
            difficulty::MIN_DIFFICULTY as i64,
            difficulty::MAX_DIFFICULTY as i64,
        ) as u32
    }

    pub fn set_active_envelopes(
        &mut self,
        env: mesh_types::ProtocolEnvelopes,
    ) -> Result<(), ChainError> {
        self.store.set_active_envelopes(env)
    }

    /// Snap stored interval **20 → 15**. Old binaries defaulted to 20 and
    /// rejected seed block 150. Skip when `MESH_FORCE_RETARGET_INTERVAL` is set.
    pub fn heal_legacy_retarget_interval(&mut self) -> Result<bool, ChainError> {
        if std::env::var("MESH_FORCE_RETARGET_INTERVAL").is_ok() {
            return Ok(false);
        }
        let mut env = self.store.active_envelopes().clone();
        if env.retarget_interval != 20 {
            return Ok(false);
        }
        env.retarget_interval = 15;
        self.store.set_active_envelopes(env)?;
        Ok(true)
    }

    /// One-shot align: `MESH_FORCE_RETARGET_INTERVAL` (10..=40).
    /// Used to heal a seed that locally drifted off the live chain's interval.
    pub fn apply_env_retarget_override(&mut self) -> Result<bool, ChainError> {
        let Ok(raw) = std::env::var("MESH_FORCE_RETARGET_INTERVAL") else {
            return Ok(false);
        };
        let Ok(n) = raw.trim().parse::<u64>() else {
            return Ok(false);
        };
        if !(10..=40).contains(&n) {
            return Ok(false);
        }
        let mut env = self.store.active_envelopes().clone();
        if env.retarget_interval == n {
            return Ok(false);
        }
        env.retarget_interval = n;
        self.store.set_active_envelopes(env)?;
        Ok(true)
    }

    pub fn height(&self) -> u64 {
        self.store.height()
    }

    pub fn finalized_height(&self) -> u64 {
        self.finality.finalized_height
    }

    #[cfg(test)]
    pub(crate) fn test_lock_tip_finality(&mut self) {
        if let Some(tip) = self.tip() {
            self.finality.finalized_height = tip.header.height;
            self.finality.finalized_hash = tip.id();
        }
    }

    pub fn finalized_hash(&self) -> Hash {
        self.finality.finalized_hash
    }

    pub fn is_finality_attestor(&self, address: &mesh_types::Address) -> bool {
        let Some(rec) = self.store.bond_for(address) else {
            return false;
        };
        crate::finality::is_finality_attestor(rec, self.height())
    }

    fn persist_finality(&self) -> Result<(), ChainError> {
        self.finality
            .persist(self.store.path(), self.genesis_hash())
    }

    fn pop_tip_checked(&mut self) -> Result<Block, ChainError> {
        let ancestor = self.height().saturating_sub(1);
        if self.finality.would_pop_finalized(ancestor) {
            return Err(ChainError::InvalidBlock(
                "cannot pop a finalized height".into(),
            ));
        }
        self.store.pop_tip()
    }

    pub fn pending_finality_attestations(&self) -> Vec<FinalityAttestation> {
        let mut out = Vec::new();
        for m in self.finality.pending.values() {
            out.extend(m.values().cloned());
        }
        out.truncate(64);
        out
    }

    pub fn finality_status(&self) -> serde_json::Value {
        let tip = self.height();
        let active = finality_active_at(tip);
        let window = finality_window();
        let candidate_h = tip.saturating_sub(window);
        let candidate = if active && tip >= window {
            self.get_block(candidate_h)
        } else {
            None
        };
        let pending = candidate
            .as_ref()
            .and_then(|b| {
                self.finality
                    .pending
                    .get(&(b.header.height, b.id()))
                    .map(|m| m.len())
            })
            .unwrap_or(0);
        let bonds = self.store.node_bonds();
        serde_json::json!({
            "active": active,
            "activation_height": finality_activation_height(),
            "window": window,
            "min_attestors": finality_min_attestors(),
            "min_bond_atomic": min_finality_bond_atomic(),
            "bond_age": finality_bond_age(),
            "genesis": self.genesis_hash().to_hex(),
            "finalized_height": self.finality.finalized_height,
            "finalized_hash": self.finality.finalized_hash.to_hex(),
            "candidate_height": candidate.as_ref().map(|b| b.header.height),
            "candidate_hash": candidate.as_ref().map(|b| b.id().to_hex()),
            "pending_attestors": pending,
            "live_attestors": crate::finality::live_finality_count(bonds, tip),
            "pending": self.pending_finality_attestations(),
            "note": if active {
                "lab finality — refuse reorgs below finalized_height"
            } else {
                "off (MESH_FINALITY_HEIGHT unset / MAX) — Fusion tip only"
            },
        })
    }

    /// Signed checkpoint. Equivocation slashes. Votes persist + are gossipable.
    pub fn record_finality_attestation(
        &mut self,
        att: FinalityAttestation,
    ) -> Result<FinalityIngest, ChainError> {
        if !att.verify() {
            return Err(ChainError::InvalidBlock("bad finality signature".into()));
        }
        if att.genesis != self.genesis_hash() {
            return Err(ChainError::InvalidBlock("finality genesis mismatch".into()));
        }
        let Some(op) = att.operator() else {
            return Err(ChainError::InvalidBlock("bad finality pubkey".into()));
        };
        if !self.is_finality_attestor(&op) {
            return Err(ChainError::InvalidBlock(
                "finality attestor must hold the finality bond floor + age".into(),
            ));
        }
        let tip = self.height();
        let key = (att.height, att.block_hash);
        let op_hex = op.to_hex();
        // Slash a flip even after that height is already finalized.
        let flip = self.finality.pending.iter().any(|((h, hash), m)| {
            *h == att.height && *hash != att.block_hash && m.contains_key(&op_hex)
        });
        if flip {
            let _ = self.slash_node_bond(op);
            self.finality.prune(tip);
            let _ = self.persist_finality();
            return Ok(FinalityIngest {
                new_vote: false,
                advanced: false,
                slashed: Some(op),
            });
        }
        if !crate::finality::attest_height_ok(att.height, tip, self.finality.finalized_height) {
            return Err(ChainError::InvalidBlock(
                "finality height not in the attest window".into(),
            ));
        }
        let Some(block) = self.get_block(att.height) else {
            return Err(ChainError::InvalidBlock("finality height unknown".into()));
        };
        if block.id() != att.block_hash {
            return Err(ChainError::InvalidBlock("finality hash mismatch".into()));
        }
        if self
            .finality
            .pending
            .get(&key)
            .and_then(|m| m.get(&op_hex))
            .is_some()
        {
            return Ok(FinalityIngest::default());
        }
        {
            let slot = self.finality.pending.entry(key).or_default();
            if slot.len() >= crate::finality::max_votes_per_height() {
                return Err(ChainError::InvalidBlock("finality vote flood".into()));
            }
            slot.insert(op_hex, att);
        }
        self.finality.prune(tip);
        self.persist_finality()?;
        let advanced = self.maybe_advance_finality();
        Ok(FinalityIngest {
            new_vote: true,
            advanced,
            slashed: None,
        })
    }

    /// Local bonded wallet signs every catch-up height through the candidate.
    pub fn maybe_local_attest(
        &mut self,
        kp: &mesh_crypto::Keypair,
    ) -> Result<Vec<FinalityAttestation>, ChainError> {
        let tip = self.height();
        if !finality_active_at(tip) {
            return Ok(Vec::new());
        }
        if !self.is_finality_attestor(&kp.address()) {
            return Ok(Vec::new());
        }
        let window = finality_window();
        if tip < window {
            return Ok(Vec::new());
        }
        let cand = tip - window;
        let from = self
            .finality
            .finalized_height
            .saturating_add(1)
            .max(cand.saturating_sub(64));
        let genesis = self.genesis_hash();
        let mut out = Vec::new();
        for h in from..=cand {
            let Some(block) = self.get_block(h) else {
                continue;
            };
            let att = FinalityAttestation::sign(kp, genesis, h, block.id());
            match self.record_finality_attestation(att.clone()) {
                Ok(ing) if ing.new_vote => out.push(att),
                Ok(_) => {}
                Err(_) => {}
            }
        }
        Ok(out)
    }

    fn maybe_advance_finality(&mut self) -> bool {
        let tip_h = self.height();
        if !finality_active_at(tip_h) {
            return false;
        }
        let window = finality_window();
        if tip_h < window {
            return false;
        }
        let candidate_h = tip_h - window;
        if candidate_h <= self.finality.finalized_height {
            return false;
        }
        let Some(block) = self.get_block(candidate_h) else {
            return false;
        };
        let cand_id = block.id();
        let key = (block.header.height, cand_id);
        let (n, attestor_weight, total) = {
            let Some(votes) = self.finality.pending.get(&key) else {
                return false;
            };
            let bonds = self.store.node_bonds();
            let mut attestor_weight = 0u64;
            let mut n = 0u64;
            for addr in votes.keys() {
                if let Some(rec) = bonds.get(addr) {
                    if !crate::finality::is_finality_attestor(rec, tip_h) {
                        continue;
                    }
                    n += 1;
                    attestor_weight = attestor_weight.saturating_add(rec.locked_atomic());
                }
            }
            (
                n,
                attestor_weight,
                crate::finality::live_finality_weight(bonds, tip_h),
            )
        };
        if n < finality_min_attestors() {
            return false;
        }
        if total == 0 {
            return false;
        }
        let bps = attestor_weight.saturating_mul(10_000) / total;
        if bps < u64::from(crate::finality::finality_threshold_bps()) {
            return false;
        }
        let prev_h = self.finality.finalized_height;
        let prev_hash = self.finality.finalized_hash;
        self.finality.finalized_height = candidate_h;
        self.finality.finalized_hash = cand_id;
        if let Err(e) = self.persist_finality() {
            self.finality.finalized_height = prev_h;
            self.finality.finalized_hash = prev_hash;
            tracing::error!(error = %e, "finality persist failed — not advancing");
            return false;
        }
        tracing::info!(
            height = candidate_h,
            hash = %self.finality.finalized_hash,
            attestors = n,
            "finality: checkpoint locked"
        );
        true
    }

    pub fn tip(&self) -> Option<Block> {
        self.store.tip()
    }

    pub fn tip_hash(&self) -> Hash {
        self.store.tip().map(|b| b.id()).unwrap_or_else(Hash::zero)
    }

    pub fn get_block(&self, height: u64) -> Option<Block> {
        self.store.get_by_height(height)
    }

    pub fn balance(&self, address: &Address) -> Amount {
        self.store.balance(address)
    }

    pub fn utxos_for(&self, address: &Address) -> Vec<(OutPoint, Utxo)> {
        self.store.utxos_for(address)
    }

    /// UTXOs that can be spent in the next block (mature coinbase, not bond-locked).
    pub fn mature_utxos_for(&self, address: &Address) -> Vec<(OutPoint, Utxo)> {
        let cbs = self.recent_coinbase_heights();
        let next_h = self.height().saturating_add(1);
        self.utxos_for(address)
            .into_iter()
            .filter(|(op, _)| {
                let mature = match cbs.get(op) {
                    Some(&h) => next_h >= h.saturating_add(COINBASE_MATURITY),
                    None => true,
                };
                mature && !self.store.is_outpoint_locked(op)
            })
            .collect()
    }

    pub fn mature_balance(&self, address: &Address) -> Amount {
        self.mature_utxos_for(address)
            .into_iter()
            .fold(Amount::ZERO, |acc, (_, u)| acc.checked_add(u.amount).unwrap_or(acc))
    }

    pub fn mempool(&self) -> &[Transaction] {
        self.store.mempool()
    }

    /// Params for the next block to mine (tip + 1).
    pub fn params(&self) -> MeshHashParams {
        self.params_at_height(self.height().saturating_add(1))
    }

    /// Consensus MeshHash params for a block at `height`.
    pub fn params_at_height(&self, height: u64) -> MeshHashParams {
        if self.light_pow {
            return MeshHashParams::light();
        }
        if meshhash_cpu::evo_active(height) {
            let seed = self.evo_period_seed(height);
            let mut p = meshhash_cpu::EvoRecipe::derive(height, &seed).params();
            if meshhash_cpu::fusion_sequential_active(height) {
                p.version = 5;
            } else if meshhash_cpu::fusion_active(height) {
                p.version = 4;
            }
            return p;
        }
        MeshHashParams::for_pow(false, height)
    }

    /// Previous block id (hashrate recycle). Genesis / height 0 → zero hash.
    pub fn evo_period_seed(&self, height: u64) -> mesh_types::Hash {
        if height == 0 {
            return mesh_types::Hash::zero();
        }
        self.store
            .tip()
            .map(|b| b.id())
            .unwrap_or_else(|| self.store.genesis_hash())
    }

    pub fn evo_recipe_at(&self, height: u64) -> Option<meshhash_cpu::EvoRecipe> {
        if self.light_pow || !meshhash_cpu::evo_active(height) {
            return None;
        }
        Some(meshhash_cpu::EvoRecipe::derive(height, &self.evo_period_seed(height)))
    }

    /// Recent verified AI receipts + tip height — informational mesh strength (Build/31).
    pub fn mesh_strength(&self) -> u64 {
        let receipts = self.store.ai_receipts().len() as u64;
        receipts.saturating_add(self.height().min(10_000))
    }

    /// PoW profile version for templates (`1`, `2`, or `3`). Light mining reports `1`.
    pub fn pow_version_at_height(&self, height: u64) -> u8 {
        self.params_at_height(height).version
    }

    /// Submit a signed transaction to the local mempool.
    pub fn submit_tx(&mut self, tx: Transaction) -> Result<Hash, ChainError> {
        if self.tx_spends_locked(&tx) && !self.is_valid_slash_settle(&tx) {
            return Err(ChainError::InvalidTx(
                "input is locked as node bond collateral (use slash settle)".into(),
            ));
        }
        let id = tx.txid();
        // Idempotent: already in mempool.
        if self.store.mempool().iter().any(|t| t.txid() == id) {
            return Ok(id);
        }
        if let Some(conflict) = self.store.mempool_input_conflict(&tx).cloned() {
            if conflict.txid() == id {
                return Ok(id);
            }
            if self.should_replace_mempool_conflict(&tx, &conflict) {
                let n = self.store.mempool_evict_input_conflicts(&tx);
                tracing::info!(
                    txid = %id,
                    evicted = n,
                    lost = %conflict.txid(),
                    "mempool replaced conflicting tx (preferred slash settle)"
                );
            } else {
                return Err(ChainError::InvalidTx(format!(
                    "mempool input conflict with {}",
                    conflict.txid()
                )));
            }
        }
        validate_mempool_tx(&tx, self.store.utxos(), self.store.mempool())?;
        self.store.mempool_push(tx)?;
        tracing::info!(txid = %id, "tx accepted to mempool");
        Ok(id)
    }

    /// Prefer SlashMark-advertised settle over a racing conflicting settle; else first-seen wins.
    fn should_replace_mempool_conflict(&self, new: &Transaction, existing: &Transaction) -> bool {
        let Some(addr_s) = new.parse_slash_settle_memo() else {
            return false;
        };
        let Some(addr) = Address::from_hex(&addr_s) else {
            return false;
        };
        if !existing.is_slash_settle() {
            // Prefer valid slash settle over a non-settle that somehow races locked outs.
            return self.is_valid_slash_settle(new);
        }
        let Some(pref) = self.store.preferred_slash_settle(&addr) else {
            return false;
        };
        let new_hex = new.txid().to_hex();
        let old_hex = existing.txid().to_hex();
        new_hex == pref && old_hex != pref
    }

    /// Build + submit slash settle for a soft-slashed bond owned by `keypair`.
    pub fn submit_slash_settle(&mut self, keypair: &Keypair) -> Result<Hash, ChainError> {
        let addr = keypair.address();
        // Honor existing preferred / racing settle already in mempool (Build/27 N5 race).
        if let Some(pref) = self.store.preferred_slash_settle(&addr) {
            if let Some(t) = self
                .store
                .mempool()
                .iter()
                .find(|t| t.txid().to_hex() == pref)
            {
                return Ok(t.txid());
            }
        }
        if let Some(t) = self.store.mempool().iter().find(|t| {
            t.parse_slash_settle_memo()
                .as_deref()
                .map(|s| Address::from_hex(s).map(|a| a == addr).unwrap_or(false))
                .unwrap_or(false)
        }) {
            return Ok(t.txid());
        }
        let bond = self
            .store
            .bond_for(&addr)
            .cloned()
            .ok_or_else(|| ChainError::InvalidTx("no bond".into()))?;
        if bond.locked.is_empty() {
            return Err(ChainError::InvalidTx("bond locks already settled".into()));
        }
        // Soft-slash if needed so eligibility drops before the settle confirms.
        if !bond.slashed {
            let _ = self.slash_node_bond(addr)?;
        }
        let bond = self
            .store
            .bond_for(&addr)
            .cloned()
            .ok_or_else(|| ChainError::InvalidTx("no bond".into()))?;
        let mut locked = Vec::new();
        for l in &bond.locked {
            let txid = Hash::from_hex(&l.txid_hex)
                .map_err(|_| ChainError::InvalidTx("bad locked txid".into()))?;
            let op = OutPoint::new(txid, l.vout);
            let u = self
                .store
                .utxos()
                .get(&op)
                .cloned()
                .ok_or_else(|| ChainError::InvalidTx(format!("locked utxo missing {op}")))?;
            locked.push((op, u));
        }
        let tx = build_slash_settle(keypair, &locked)?;
        // Advertise our settle as preferred before push so peers/local races defer to it.
        self.store
            .set_preferred_slash_settle(&addr, &tx.txid().to_hex());
        self.submit_tx(tx)
    }

    /// Build and submit a payment from `keypair` to `to`.
    pub fn send(
        &mut self,
        keypair: &Keypair,
        to: Address,
        amount: Amount,
        memo: impl Into<String>,
    ) -> Result<Hash, ChainError> {
        let utxos = self.store.spendable_utxos_for(&keypair.address());
        let tx = build_signed_payment(keypair, &utxos, to, amount, memo)?;
        self.submit_tx(tx)
    }

    /// Build an unsolved block template for the next height (coinbase + mempool).
    pub fn mining_template(&self, miner: Address) -> Block {
        let difficulty = self.next_difficulty();
        let height = if self.store.tip().is_some() {
            self.height() + 1
        } else {
            0
        };
        let prev_hash = self.tip_hash();
        let exam_scores = self.store.gpu_scores();
        // Clean 45/45/10: GPU lane is the finder's work. Helper floor (if armed)
        // is the only path that pays exam scores from this pot.
        let gpu_paid = if mesh_types::helper_floor_active(height) {
            exam_scores.clone()
        } else if mesh_types::fair_lane_split_active(height) {
            gpu_scores_with_fusion_credit(height, miner, &HashMap::new())
        } else {
            gpu_scores_with_fusion_credit(height, miner, exam_scores)
        };
        let coinbase = build_market_coinbase(
            height,
            miner,
            &gpu_paid,
            self.store.node_scores(),
        );

        let mut txs = vec![coinbase];
        let mut view = self.store.utxo_snapshot();
        let cbs = self.recent_coinbase_heights();
        for pending in self.store.mempool() {
            if txs.len() > MAX_BLOCK_TXS {
                break;
            }
            if self.tx_spends_locked(pending) && !self.is_valid_slash_settle(pending) {
                continue;
            }
            if validate::validate_tx_at_height(pending, &view, &cbs, height).is_ok() {
                let _ = apply_pending(&mut view, pending);
                txs.push(pending.clone());
            }
        }

        let merkle_root = Block::merkle_root(&txs);
        let timestamp = {
            let now = now_secs();
            match self.store.tip() {
                Some(t) => now.max(t.header.timestamp.saturating_add(1)),
                None => now.max(1),
            }
        };

        Block {
            header: BlockHeader {
                version: 1,
                prev_hash,
                merkle_root,
                timestamp,
                height,
                difficulty,
                nonce: 0,
            },
            txs,
        }
    }

    /// CPU-bound PoW search on a prepared template. Safe to run off-thread;
    /// does not touch chain state.
    pub fn search_pow(block: &mut Block, light_pow: bool, max_nonces: u64) -> bool {
        let difficulty = block.header.difficulty;
        let commitment = block.header.pre_pow_commitment();
        let height = block.header.height;
        let prev = block.header.prev_hash;
        for nonce in 0..max_nonces {
            block.header.nonce = nonce;
            let pow = pow_hash_header(&commitment, nonce, light_pow, height, &prev);
            if pow.meets_difficulty(difficulty) {
                return true;
            }
        }
        false
    }

    /// Validate and append a mined block. Uses the same fork-choice as P2P
    /// (`import_block`) so a better-work sibling is not discarded as "stale".
    /// Returns `Ok(None)` if the block is already the tip or loses the race.
    pub fn accept_mined(&mut self, block: Block) -> Result<Option<Block>, ChainError> {
        let id = block.id();
        let height = block.header.height;
        let nonce = block.header.nonce;
        let difficulty = block.header.difficulty;
        let mempool_n = block.txs.len().saturating_sub(1);
        if !self.import_block(block.clone())? {
            if self.tip_hash() == id {
                return Ok(None);
            }
            tracing::warn!(height, %id, "mined block not adopted (tip race / weaker fork)");
            return Ok(None);
        }
        let pow = {
            let commitment = block.header.pre_pow_commitment();
            pow_hash_header(
                &commitment,
                nonce,
                self.light_pow,
                height,
                &block.header.prev_hash,
            )
        };
        tracing::info!(
            height,
            nonce,
            difficulty,
            id = %id,
            pow = %pow,
            mempool_txs = mempool_n,
            "block mined"
        );
        Ok(Some(block))
    }

    /// Mine the next block (solo) and append if valid.
    /// Prefer the template → [`search_pow`] → [`accept_mined`] split when the
    /// chain lock must stay free for RPC/P2P.
    pub fn mine_next(
        &mut self,
        miner: Address,
        max_nonces: u64,
    ) -> Result<Option<Block>, ChainError> {
        let mut block = self.mining_template(miner);
        let light = self.light_pow;
        if !Self::search_pow(&mut block, light, max_nonces) {
            return Ok(None);
        }
        self.accept_mined(block)
    }

    pub fn store(&self) -> &ChainStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut ChainStore {
        &mut self.store
    }

    /// Credit GPU market score (from verified AiJobReceipt). Does not pay until next coinbase.
    pub fn credit_gpu_score(
        &mut self,
        address: Address,
        weight: u64,
    ) -> Result<(), ChainError> {
        self.store.credit_gpu(&address, weight)
    }

    pub fn credit_node_score(
        &mut self,
        address: Address,
        weight: u64,
    ) -> Result<(), ChainError> {
        self.store.credit_node(&address, weight)
    }

    /// Record a verified AI receipt. Dedupes by `job_id`. Returns `true` if newly stored.
    /// Soft-adapt runs only for locally verified completions (`allow_soft_adapt`).
    pub fn record_ai_receipt(
        &mut self,
        receipt: mesh_types::AiJobReceipt,
    ) -> Result<bool, ChainError> {
        self.record_ai_receipt_ex(receipt, true)
    }

    /// P2P / relay import path — credit markets but never soft-adapt under the chain lock.
    pub fn record_ai_receipt_imported(
        &mut self,
        receipt: mesh_types::AiJobReceipt,
    ) -> Result<bool, ChainError> {
        self.record_ai_receipt_ex(receipt, false)
    }

    fn record_ai_receipt_ex(
        &mut self,
        receipt: mesh_types::AiJobReceipt,
        allow_soft_adapt: bool,
    ) -> Result<bool, ChainError> {
        let worker = receipt.worker;
        let next_h = self.height().saturating_add(1);
        // GPU coinbase share is only from a locally rematched exam
        // (`allow_soft_adapt` = `/v1/exam/submit`). Gossip / `/v1/aireceipt`
        // must not mint exam units from a job_id prefix.
        let weight = if !allow_soft_adapt {
            0
        } else if mesh_types::useful_work_active(next_h) {
            if mesh_types::is_exam_job_id(&receipt.job_id) {
                mesh_types::EXAM_LANE_UNITS
            } else if mesh_types::is_paid_research_kind(receipt.job_kind) {
                mesh_types::RESEARCH_LANE_UNITS
            } else {
                0
            }
        } else if mesh_types::fair_lane_split_active(next_h) {
            if mesh_types::is_exam_job_id(&receipt.job_id) {
                mesh_types::EXAM_LANE_UNITS
            } else {
                0
            }
        } else {
            receipt.weight
        };
        let is_eval = matches!(receipt.job_kind, mesh_types::AiJobKind::ProtocolEval);
        if !self.store.push_ai_receipt(receipt)? {
            return Ok(false);
        }
        if weight > 0 {
            self.store.credit_gpu(&worker, weight)?;
        }
        if allow_soft_adapt && is_eval {
            let _ = self.maybe_auto_adapt_soft_envelopes()?;
        }
        Ok(true)
    }

    /// True if this address already MATCH'd the immune exam for `height`.
    pub fn has_exam_receipt(&self, height: u64, worker: &mesh_types::Address) -> bool {
        let id = mesh_types::exam_job_id(height, worker);
        self.store
            .ai_receipts()
            .iter()
            .any(|r| r.job_id == id)
    }

    /// Pending node-market weight for an operator address (0 if none).
    pub fn pending_node_weight(&self, address: &mesh_types::Address) -> u64 {
        self.store
            .node_scores()
            .get(&address.to_hex())
            .copied()
            .unwrap_or(0)
    }

    /// Soft envelopes auto-apply after enough verified protocol_eval receipts (Build/21).
    /// Bounded retarget knobs only move when quantum_grover certificates clear the gate (Build/30).
    /// BPS suggestions are recorded but never applied to emission.
    pub fn maybe_auto_adapt_soft_envelopes(
        &mut self,
    ) -> Result<Option<mesh_types::ParamProposal>, ChainError> {
        const MIN_EVALS: u64 = 3;
        let eval_count = self.store.protocol_eval_receipt_count();
        let since = eval_count.saturating_sub(self.store.last_auto_adapt_eval_count());
        let grover_n = self.store.grover_eval_count();
        let grover_since = self.store.grover_certs_since_retarget_adapt();
        let soft_ready = eval_count >= MIN_EVALS && since >= MIN_EVALS;
        let retarget_ready = grover_since >= mesh_types::MIN_GROVER_CERTS_FOR_RETARGET;
        if !soft_ready && !retarget_ready {
            return Ok(None);
        }
        let gpu_w: u64 = self.store.gpu_scores().values().sum();
        let node_w: u64 = self.store.node_scores().values().sum();
        let pulse = mesh_ai_pulse_shim(
            self.height(),
            self.tip_hash().to_string(),
            gpu_w,
            node_w,
            self.store.ai_receipts(),
        );
        let prev = self.store.active_envelopes().clone();
        let mut proposal =
            local_propose_from_pulse(&pulse, self.store.proposals().len() as u64 + 1);
        // Soft knobs only. Local AI must never move consensus retarget
        // (interval/step/floor). That forked seed vs edge at height 1720:
        // same tip, seed interval 20 expected diff 9, edge interval 15 kept 8.
        proposal.envelopes.freeze_retarget_from(&prev);
        if retarget_ready {
            proposal.rationale = format!(
                "{}; grover certs={grover_n} since={grover_since} (retarget frozen — not a block clock)",
                proposal.rationale
            );
        }
        proposal.rationale = format!("auto-adapt: {}", proposal.rationale);
        let applied =
            self.store
                .apply_soft_auto_adapt(proposal, self.height(), eval_count)?;
        Ok(Some(applied))
    }

    /// Credit this node's operator for useful relay (Build/06 / Build/14).
    /// Soft: `idle_stipend_bps_cap` scales the credit (tighter stipend → less idle node pay).
    /// Anti-Sybil: requires node bond + min liquid stake (Build/27 N5).
    pub fn credit_local_relay(&mut self, weight: u64) -> Result<(), ChainError> {
        self.credit_local_service(mesh_types::NodeServiceKind::TxRelay, weight)
    }

    /// Soft node reputation (milli, 1000 = full) from recent service diversity (Build/27 B8).
    /// No attested work → 0 (idle nodes do not look fully useful); 1 kind → 600; 2 → 800; 3+ → 1000.
    pub fn node_reputation_milli(&self, address: &mesh_types::Address) -> u64 {
        let mut kinds = std::collections::HashSet::new();
        for a in self.store.service_attestations() {
            if &a.operator == address {
                kinds.insert(a.service);
            }
        }
        match kinds.len() {
            0 => 0,
            1 => 600,
            2 => 800,
            _ => 1_000,
        }
    }

    /// Change who receives node-market credits for later useful work.
    pub fn set_node_operator(&mut self, address: Address) {
        self.node_operator = Some(address);
    }

    /// Update local mesh RTT dampener (from libp2p ping median).
    pub fn set_relay_rtt_factor_milli(&mut self, milli: u64) {
        self.relay_rtt_factor_milli = milli.clamp(100, 1_000);
    }

    /// Credit operator for a typed node service (Build/06 attestation weights).
    pub fn credit_local_service(
        &mut self,
        service: mesh_types::NodeServiceKind,
        weight: u64,
    ) -> Result<(), ChainError> {
        if weight == 0 {
            return Ok(());
        }
        let Some(op) = self.node_operator else {
            return Ok(());
        };
        let height = self.height();
        let _ = self.store.ensure_node_bond(&op, "", height)?;
        if !self.store.is_node_bond_eligible(&op)
            && std::env::var("MESH_NODE_BOND")
                .map(|v| {
                    let v = v.trim().to_ascii_lowercase();
                    !(v == "0" || v == "false" || v == "off")
                })
                .unwrap_or(true)
        {
            return Ok(());
        }
        let cap = self.store.active_envelopes().idle_stipend_bps_cap as u64;
        let service_bps = service.weight_bps();
        // Count this event toward diversity so the first useful service is not zeroed.
        let mut kinds = std::collections::HashSet::new();
        kinds.insert(service);
        for a in self.store.service_attestations() {
            if a.operator == op {
                kinds.insert(a.service);
            }
        }
        let rep = match kinds.len() {
            0 => 0,
            1 => 600,
            2 => 800,
            _ => 1_000,
        };
        let rtt = self.relay_rtt_factor_milli.max(1);
        let scaled = weight
            .saturating_mul(service_bps)
            .saturating_div(1_000)
            .saturating_mul(cap)
            .saturating_div(1_000)
            .saturating_mul(rep)
            .saturating_div(1_000)
            .saturating_mul(rtt)
            .saturating_div(1_000);
        if scaled == 0 {
            return Ok(());
        }
        self.store.credit_node(&op, scaled)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.store.push_service_attestation(mesh_types::NodeServiceAttestation {
            operator: op,
            service,
            weight,
            credited: scaled,
            attested_at: now,
        });
        Ok(())
    }

    pub fn recent_service_attestations(&self) -> Vec<mesh_types::NodeServiceAttestation> {
        self.store.service_attestations().cloned().collect()
    }

    pub fn register_node_bond(
        &mut self,
        address: mesh_types::Address,
        peer_id: &str,
    ) -> Result<NodeBondRec, ChainError> {
        let h = self.height();
        self.store.register_node_bond(&address, peer_id, h)
    }

    pub fn request_node_unbond(
        &mut self,
        address: mesh_types::Address,
    ) -> Result<NodeBondRec, ChainError> {
        let h = self.height();
        self.store.request_node_unbond(&address, h)
    }

    pub fn finalize_node_unbond(
        &mut self,
        address: mesh_types::Address,
    ) -> Result<NodeBondRec, ChainError> {
        let h = self.height();
        self.store.finalize_node_unbond(&address, h)
    }

    pub fn slash_node_bond(
        &mut self,
        address: mesh_types::Address,
    ) -> Result<NodeBondRec, ChainError> {
        let h = self.height();
        self.store.slash_node_bond(&address, h)
    }

    pub fn node_bond(&self, address: &mesh_types::Address) -> Option<NodeBondRec> {
        self.store.bond_for(address).cloned()
    }

    pub fn is_node_bond_eligible(&self, address: &mesh_types::Address) -> bool {
        self.store.is_node_bond_eligible(address)
    }

    pub fn spendable_balance(&self, address: &Address) -> Amount {
        self.store.spendable_balance(address)
    }

    /// Flush batched score writes to disk (call from periodic tick).
    pub fn flush_store(&mut self) -> Result<(), ChainError> {
        self.store.flush_if_dirty()
    }

    pub fn generate_adaptive_proposal(&mut self) -> Result<mesh_types::ParamProposal, ChainError> {
        let gpu_w: u64 = self.store.gpu_scores().values().sum();
        let node_w: u64 = self.store.node_scores().values().sum();
        let pulse = mesh_ai_pulse_shim(
            self.height(),
            self.tip_hash().to_string(),
            gpu_w,
            node_w,
            self.store.ai_receipts(),
        );
        let proposal = local_propose_from_pulse(&pulse, self.store.proposals().len() as u64 + 1);
        self.store.push_proposal(proposal)
    }

    pub fn activate_proposal(&mut self, id: &str) -> Result<mesh_types::ProtocolEnvelopes, ChainError> {
        self.store.activate_proposal(id)
    }

    pub fn reject_proposal(&mut self, id: &str) -> Result<(), ChainError> {
        self.store.reject_proposal(id)
    }

    pub fn cast_proposal_vote(
        &mut self,
        id: &str,
        node_id: &str,
        choice: mesh_types::VoteChoice,
    ) -> Result<mesh_types::ParamProposal, ChainError> {
        self.store
            .cast_proposal_vote(id, node_id, choice, self.height())
    }

    pub fn active_envelopes(&self) -> mesh_types::ProtocolEnvelopes {
        self.store.active_envelopes().clone()
    }

    pub fn last_auto_adapt_at_height(&self) -> u64 {
        self.store.last_auto_adapt_at_height()
    }

    pub fn last_auto_adapt_proposal_id(&self) -> String {
        self.store.last_auto_adapt_proposal_id().to_string()
    }

    pub fn last_auto_adapt_eval_count(&self) -> u64 {
        self.store.last_auto_adapt_eval_count()
    }

    pub fn grover_eval_count(&self) -> u64 {
        self.store.grover_eval_count()
    }

    pub fn grover_certs_since_retarget_adapt(&self) -> u64 {
        self.store.grover_certs_since_retarget_adapt()
    }

    pub fn last_retarget_adapt_grover_count(&self) -> u64 {
        self.store.last_retarget_adapt_grover_count()
    }

    pub fn improvement_certs(&self) -> &[mesh_types::ImprovementCertificate] {
        self.store.improvement_certs()
    }

    pub fn param_epoch(&self) -> u64 {
        self.store.param_epoch()
    }

    pub fn epoch_history(&self) -> &[mesh_types::ParamEpoch] {
        self.store.epoch_history()
    }

    pub fn proposals(&self) -> &[mesh_types::ParamProposal] {
        self.store.proposals()
    }
}

/// Local MeshPulse-shaped inputs for proposal generation (no mesh-ai dep in chain).
struct PulseIn {
    height: u64,
    gpu_vs_height_signal: f64,
    avg_latency_ms: f64,
    pending_node_weight: u64,
    gpu_receipts: usize,
    research_eval_receipts: u64,
    research_progress: f64,
    echo_ok_rate: f64,
    mean_primary: f64,
    mean_orphan_risk: f64,
    mean_detect_rate: f64,
    mean_linkability: f64,
    mean_backlog_ratio: f64,
    mean_latency_p95_ms: f64,
    /// Mean primary across quantum_* ProtocolEval receipts (0 if none).
    quantum_mean_primary: f64,
    quantum_receipts: u64,
    /// Mean primary for quantum_grover only (Build/30).
    grover_mean_primary: f64,
    grover_receipts: u64,
}

fn mesh_ai_pulse_shim(
    height: u64,
    _tip: String,
    pending_gpu_weight: u64,
    pending_node_weight: u64,
    receipts: &[mesh_types::AiJobReceipt],
) -> PulseIn {
    let n = receipts.len();
    let avg_latency = if n == 0 {
        0.0
    } else {
        receipts.iter().map(|r| r.latency_ms as f64).sum::<f64>() / n as f64
    };
    let evals: Vec<_> = receipts
        .iter()
        .filter(|r| matches!(r.job_kind, mesh_types::AiJobKind::ProtocolEval))
        .collect();
    let eval_count = evals.len() as u64;
    let mut scenarios = std::collections::BTreeSet::new();
    for r in &evals {
        if !r.research_scenario.is_empty() {
            scenarios.insert(r.research_scenario.as_str());
        }
    }
    let touched = scenarios.len() as f64;
    // Classical (8) + quantum (3) scenarios.
    let catalog = 11.0;
    let coverage = (touched / catalog).min(1.0);
    let volume = (eval_count as f64 / 10.0).min(1.0);
    let research_progress = 0.6 * coverage + 0.4 * volume;
    let en = evals.len() as f64;
    let (mean_primary, mean_orphan_risk, mean_detect_rate, mean_linkability, mean_backlog_ratio, mean_latency_p95_ms) =
        if en <= 0.0 {
            (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        } else {
            (
                evals.iter().map(|r| r.score_primary).sum::<f64>() / en,
                evals.iter().map(|r| r.score_orphan_risk).sum::<f64>() / en,
                evals.iter().map(|r| r.score_detect_rate).sum::<f64>() / en,
                evals.iter().map(|r| r.score_linkability).sum::<f64>() / en,
                evals.iter().map(|r| r.score_backlog_ratio).sum::<f64>() / en,
                evals.iter().map(|r| r.score_latency_p95_ms).sum::<f64>() / en,
            )
        };
    let q_evals: Vec<_> = evals
        .iter()
        .filter(|r| r.research_scenario.starts_with("quantum_"))
        .collect();
    let qn = q_evals.len() as f64;
    let quantum_mean_primary = if qn <= 0.0 {
        0.0
    } else {
        q_evals.iter().map(|r| r.score_primary).sum::<f64>() / qn
    };
    let g_evals: Vec<_> = evals
        .iter()
        .filter(|r| r.research_scenario == "quantum_grover")
        .collect();
    let gn = g_evals.len() as f64;
    let grover_mean_primary = if gn <= 0.0 {
        0.0
    } else {
        g_evals.iter().map(|r| r.score_primary).sum::<f64>() / gn
    };
    PulseIn {
        height,
        gpu_vs_height_signal: pending_gpu_weight as f64 / (height.max(1) as f64),
        avg_latency_ms: avg_latency,
        pending_node_weight,
        gpu_receipts: n,
        research_eval_receipts: eval_count,
        research_progress,
        echo_ok_rate: 1.0,
        mean_primary,
        mean_orphan_risk,
        mean_detect_rate,
        mean_linkability,
        mean_backlog_ratio,
        mean_latency_p95_ms,
        quantum_mean_primary,
        quantum_receipts: q_evals.len() as u64,
        grover_mean_primary,
        grover_receipts: g_evals.len() as u64,
    }
}

fn local_propose_from_pulse(pulse: &PulseIn, next_id: u64) -> mesh_types::ParamProposal {
    use mesh_types::{
        ProposalStatus, ProtocolEnvelopes, BPS_CEIL_CPU, BPS_CEIL_GPU, BPS_CEIL_NODE,
        BPS_FLOOR_CPU, BPS_FLOOR_GPU, BPS_FLOOR_NODE,
    };
    let mut env = ProtocolEnvelopes::default();
    let mut rationale = Vec::new();
    if pulse.gpu_vs_height_signal < 0.3 {
        env.soft_adapt_signal_threshold = 0.8;
        env.soft_benchmark_rounds = 5_000;
        rationale.push("low GPU signal → propose more GPU workload");
    } else if pulse.gpu_vs_height_signal > 2.0 {
        env.soft_adapt_signal_threshold = 0.2;
        env.soft_benchmark_rounds = 500;
        rationale.push("high GPU backlog → propose cooler soft-adapt");
    }
    if pulse.avg_latency_ms > 2_000.0 || pulse.mean_latency_p95_ms > 1_500.0 {
        env.min_verifier_weight = 2;
        rationale.push("high latency → raise verifier weight floor");
    }
    if pulse.mean_detect_rate > 0.0 && pulse.mean_detect_rate < 0.7 {
        env.min_verifier_weight = env.min_verifier_weight.max(3);
        rationale.push("security sim weak detect_rate → raise min verifier weight");
    }
    if pulse.mean_orphan_risk > 0.45 {
        env.soft_adapt_signal_threshold = env.soft_adapt_signal_threshold.max(0.6);
        env.suggested_cpu_diff_bias = -1;
        rationale.push("orphan risk high → cooler adapt + soft CPU ease");
    }
    if pulse.mean_linkability > 0.55 {
        env.idle_stipend_bps_cap = env.idle_stipend_bps_cap.min(750);
        rationale.push("privacy linkability elevated → tighten idle stipend");
    }
    if pulse.mean_backlog_ratio > 0.55 {
        env.soft_benchmark_rounds = env.soft_benchmark_rounds.max(4_000);
        rationale.push("scale backlog high → more GPU research budget");
    }
    if pulse.research_progress < 0.2 && pulse.height > 5 {
        env.soft_benchmark_rounds = env.soft_benchmark_rounds.max(3_000);
        rationale.push("low research progress → more GPU research budget");
    } else if pulse.research_progress > 0.7 && pulse.mean_primary > 0.65 {
        env.suggested_cpu_diff_bias = 0;
        rationale.push("healthy research coverage — keep soft envelopes steady");
    }
    if pulse.echo_ok_rate < 0.9 {
        env.min_verifier_weight = env.min_verifier_weight.max(2);
        rationale.push("verify failures → raise min verifier weight");
    }
    if pulse.pending_node_weight == 0 && pulse.height > 10 {
        rationale.push("node score empty — keep node vault filling via relay credits");
    }
    if pulse.gpu_receipts == 0 && pulse.height > 5 {
        env.idle_stipend_bps_cap = 500;
        rationale.push("no GPU receipts yet — keep idle stipend tight");
    }
    if pulse.mean_primary > 0.0 && pulse.mean_primary < 0.45 {
        env.soft_benchmark_rounds = env.soft_benchmark_rounds.max(5_000);
        rationale.push("low mean research primary → intensify protocol sims");
    }
    // Network growth self-tune: taller chain expects more AI coverage.
    if pulse.height > 1_000 && pulse.gpu_vs_height_signal < 0.5 {
        env.soft_adapt_signal_threshold = env.soft_adapt_signal_threshold.max(0.7);
        env.soft_benchmark_rounds = env.soft_benchmark_rounds.max(4_000);
        rationale.push("network growth with thin AI → demand more GPU coverage");
    }
    if pulse.height > 5_000 && pulse.research_eval_receipts < pulse.height / 20 {
        env.soft_benchmark_rounds = env.soft_benchmark_rounds.max(6_000);
        env.idle_stipend_bps_cap = env.idle_stipend_bps_cap.max(1_000);
        rationale.push("large chain, sparse research history → raise research budget");
    }
    if pulse.height > 500 && pulse.mean_orphan_risk > 0.35 && pulse.mean_backlog_ratio > 0.4 {
        env.min_verifier_weight = env.min_verifier_weight.max(3);
        env.suggested_cpu_diff_bias = -1;
        rationale.push("growth stress (orphan+backlog) → stricter verify + soft CPU ease");
    }

    // Quantum readiness (Build/26) — soft intensity only.
    if pulse.quantum_receipts > 0 && pulse.quantum_mean_primary < 0.45 {
        env.quantum_train_enable = 1;
        env.quantum_parallel = env.quantum_parallel.max(2);
        env.soft_benchmark_rounds = env.soft_benchmark_rounds.max(4_000);
        rationale.push(
            "quantum pressure-tests weak → more quantum guardian training + sim budget",
        );
    } else if pulse.quantum_receipts == 0 && pulse.height > 100 {
        env.quantum_train_enable = 1;
        env.quantum_parallel = env.quantum_parallel.max(1);
        rationale.push("no quantum_* receipts yet → keep quantum research enabled");
    }

    // Build/30: quantum_grover → bounded retarget posture (gate applied by caller).
    if pulse.grover_receipts > 0 {
        if pulse.grover_mean_primary < 0.45 {
            env.min_difficulty_floor = env.min_difficulty_floor.max(8);
            env.retarget_step = 2;
            env.suggested_cpu_diff_bias = env.suggested_cpu_diff_bias.max(0);
            rationale.push(
                "quantum_grover weak → harden min difficulty floor + retarget step (gated)",
            );
        } else if pulse.grover_mean_primary >= 0.65 && pulse.mean_orphan_risk < 0.25 {
            // Healthy grover + stable mesh → allow slightly tighter retarget cadence.
            env.retarget_interval = 15;
            rationale.push(
                "quantum_grover healthy + stable mesh → tighten retarget interval (gated)",
            );
        }
    }

    // Absolute-best soft performance: healthy mesh → ease soft mining hint + full stipend.
    // Consensus difficulty / block time / BPS stay locked except gated retarget above.
    let healthy = pulse.mean_orphan_risk < 0.25
        && pulse.mean_backlog_ratio < 0.45
        && pulse.echo_ok_rate >= 0.95
        && (pulse.mean_primary <= 0.0 || pulse.mean_primary >= 0.5);
    if healthy && pulse.height > 50 {
        env.brain_prefer_v2 = 1;
        env.idle_stipend_bps_cap = env.idle_stipend_bps_cap.max(1_000);
        if pulse.mean_orphan_risk < 0.15 && pulse.research_progress > 0.45 {
            env.suggested_cpu_diff_bias = -2;
            rationale.push(
                "peak soft performance — max soft CPU ease (BPS/crypto unchanged)",
            );
        } else {
            env.suggested_cpu_diff_bias = env.suggested_cpu_diff_bias.min(-1);
            rationale.push(
                "healthy mesh → soft CPU ease + full research stipend for best throughput",
            );
        }
    }

    if pulse.research_eval_receipts > 0 {
        rationale.push("protocol_eval receipts feeding soft auto-adapt");
    }
    if rationale.is_empty() {
        rationale.push("steady MeshPulse — propose default envelopes");
    }
    mesh_types::ParamProposal {
        id: format!("prop-{next_id}"),
        created_at_height: pulse.height,
        rationale: rationale.join("; "),
        envelopes: env.clamp(),
        status: ProposalStatus::Pending,
        suggested_cpu_bps: 4_000u16.clamp(BPS_FLOOR_CPU, BPS_CEIL_CPU),
        suggested_gpu_bps: 4_000u16.clamp(BPS_FLOOR_GPU, BPS_CEIL_GPU),
        suggested_node_bps: 2_000u16.clamp(BPS_FLOOR_NODE, BPS_CEIL_NODE),
        votes: Vec::new(),
    }
}

fn apply_pending(
    utxos: &mut std::collections::HashMap<OutPoint, Utxo>,
    tx: &Transaction,
) -> Result<(), ChainError> {
    store::apply_tx_utxos(utxos, tx, false)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn build_genesis(light_pow: bool) -> Result<Block, ChainError> {
    let coinbase = build_market_coinbase(
        0,
        genesis_reward_address(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let txs = vec![coinbase];
    let merkle_root = Block::merkle_root(&txs);

    let mut header = BlockHeader {
        version: 1,
        prev_hash: Hash::zero(),
        merkle_root,
        timestamp: 1_700_000_000, // fixed for reproducibility
        height: 0,
        difficulty: 1, // genesis is easy
        nonce: 0,
    };

    let commitment = header.pre_pow_commitment();
    // Genesis is always MeshHash v1 (height 0), including full-PoW networks.
    let params = if light_pow {
        MeshHashParams::light()
    } else {
        MeshHashParams::v1()
    };

    for nonce in 0..u64::MAX {
        header.nonce = nonce;
        let pow = meshhash_cpu_with_params(&commitment, nonce, &params);
        if pow.meets_difficulty(1) {
            break;
        }
    }

    Ok(Block { header, txs })
}

/// Fixed address that receives the genesis CPU coinbase (deterministic across nodes).
pub fn genesis_reward_address() -> Address {
    Address::from_pubkey_bytes(b"MonkeyMesh/genesis/v1")
}

/// Deferred GPU market vault — holds unclaimed GPU 40% (Build/14).
pub fn deferred_gpu_vault() -> Address {
    mesh_types::gpu_vault_address()
}

/// Deferred node market vault — holds unclaimed Node 20% (Build/14).
pub fn deferred_node_vault() -> Address {
    mesh_types::node_vault_address()
}

/// Slash vault — destination for slashed node-bond collateral (Build/27 N5).
/// Soft slash freezes; on-chain settle (`slash:v1` memo) moves UTXOs here when mined.
pub fn deferred_slash_vault() -> Address {
    Address::from_pubkey_bytes(b"MonkeyMesh/vault/slash/v1")
}

/// Split `total` across `scores` proportionally; if empty, pay `vault` the full amount.
pub fn split_market_payouts(
    scores: &HashMap<String, u64>,
    total: Amount,
    vault: Address,
) -> Vec<(Address, Amount)> {
    let sum: u64 = scores.values().sum();
    if sum == 0 || total.atomic() == 0 {
        return vec![(vault, total)];
    }
    let mut out = Vec::new();
    let mut allocated = 0u64;
    let mut items: Vec<_> = scores.iter().collect();
    items.sort_by(|a, b| a.0.cmp(b.0));
    for (i, (addr_hex, &w)) in items.iter().enumerate() {
        let Some(addr) = Address::from_hex(addr_hex) else {
            continue;
        };
        let share = if i + 1 == items.len() {
            total.atomic().saturating_sub(allocated)
        } else {
            total.atomic().saturating_mul(w) / sum
        };
        allocated = allocated.saturating_add(share);
        if share > 0 {
            out.push((addr, Amount::from_atomic(share)));
        }
    }
    if out.is_empty() {
        vec![(vault, total)]
    } else {
        out
    }
}

/// Build PoMC multi-market coinbase for `height` (CPU miner + GPU/node ledgers or vaults).
pub fn build_market_coinbase(
    height: u64,
    cpu_miner: Address,
    gpu_scores: &HashMap<String, u64>,
    node_scores: &HashMap<String, u64>,
) -> Transaction {
    let gpu_units: u64 = gpu_scores.values().copied().sum();
    let cpu_amt = cpu_market_reward_with(height, gpu_units);
    let gpu_amt = gpu_market_reward_with(height, gpu_units);
    let node_amt = node_market_reward(height);
    let (gpu, n_exam) = if mesh_types::finder_unify_active(height) {
        (Vec::new(), 0)
    } else if mesh_types::helper_floor_active(height) {
        gpu_lane_helper_outputs(height, cpu_miner, gpu_scores, gpu_amt)
    } else if mesh_types::fair_lane_split_active(height) {
        let mut fusion = HashMap::new();
        fusion.insert(cpu_miner.to_hex(), mesh_types::FUSION_GPU_UNITS);
        (
            split_market_payouts(&fusion, gpu_amt, deferred_gpu_vault()),
            0,
        )
    } else {
        (
            split_market_payouts(gpu_scores, gpu_amt, deferred_gpu_vault()),
            0,
        )
    };
    let node = split_market_payouts(node_scores, node_amt, deferred_node_vault());
    let mut tx = Transaction::market_coinbase(height, (cpu_miner, cpu_amt), &gpu, &node);
    if n_exam > 0 {
        tx.memo = format!("{}|exam:{n_exam}", tx.memo);
    }
    if height == 0 {
        tx.memo = format!("{}|{}", tx.memo, GENESIS_MEMO);
    }
    tx
}

/// Exam helpers take `HELPER_EXAM_FLOOR_BPS` of the GPU 45%; Fusion finder takes the rest.
/// Empty exam ledger pays that floor to the GPU vault — GPU farms want CPU helpers MATCH'd.
fn gpu_lane_helper_outputs(
    height: u64,
    finder: Address,
    exam_scores: &HashMap<String, u64>,
    gpu_amt: Amount,
) -> (Vec<(Address, Amount)>, usize) {
    let exam_amt = gpu_amt.split_bps(mesh_types::HELPER_EXAM_FLOOR_BPS);
    let fusion_amt = Amount::from_atomic(gpu_amt.atomic().saturating_sub(exam_amt.atomic()));
    let exam = split_market_payouts(exam_scores, exam_amt, deferred_gpu_vault());
    let n_exam = exam.len();
    let mut fusion = HashMap::new();
    let pay_fusion = !(mesh_types::gpu_pay_requires_exam(height)
        && !exam_scores.contains_key(&finder.to_hex()));
    if pay_fusion {
        fusion.insert(finder.to_hex(), mesh_types::FUSION_GPU_UNITS);
    }
    let mut out = exam;
    out.extend(split_market_payouts(&fusion, fusion_amt, deferred_gpu_vault()));
    (out, n_exam)
}

/// Convenience: generate a throwaway miner key for demos.
pub fn demo_miner() -> (Keypair, Address) {
    let kp = Keypair::generate();
    let addr = kp.address();
    (kp, addr)
}

pub fn pow_hash(commitment: &Hash, nonce: u64, light: bool) -> Hash {
    pow_hash_at_height(commitment, nonce, light, 0)
}

pub fn pow_hash_at_height(commitment: &Hash, nonce: u64, light: bool, height: u64) -> Hash {
    pow_hash_header(commitment, nonce, light, height, &Hash::zero())
}

#[allow(dead_code)]
fn _target_block_time() -> u64 {
    TARGET_BLOCK_TIME_SECS
}
