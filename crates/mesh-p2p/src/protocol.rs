use mesh_chain::FinalityAttestation;
use mesh_types::{Block, BlockHeader, Hash, Transaction};
use serde::{Deserialize, Serialize};

/// Wire protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

/// libp2p protocol id for request-response sync.
pub const MESH_RR_PROTOCOL: &str = "/monkeymesh/sync/1.0.0";

/// Legacy gossip topic (all message kinds) — kept for older peers.
pub const MESH_GOSSIP_TOPIC: &str = "monkeymesh/1";
/// Chain blocks + txs (Build/10 topic split).
pub const MESH_GOSSIP_CHAIN: &str = "monkeymesh/chain/1";
/// AI job / result gossip.
pub const MESH_GOSSIP_AI: &str = "monkeymesh/ai/1";

/// Network messages.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NetMsg {
    Hello {
        version: u32,
        height: u64,
        tip: Hash,
        genesis: Hash,
        /// Advertised listen multiaddr (may be empty).
        listen_addr: String,
    },
    /// Reply to Hello with our chain tip (no side-effect requests).
    HelloAck {
        version: u32,
        height: u64,
        tip: Hash,
        genesis: Hash,
    },
    PeerList {
        peers: Vec<String>,
    },
    Tx(Transaction),
    Block(Block),
    /// Request blocks starting at `from_height` (inclusive), up to `limit`.
    GetBlocks {
        from_height: u64,
        limit: u32,
    },
    /// Batch response to GetBlocks.
    Blocks(Vec<Block>),
    /// Tip + block batch for catch-up (Build/10 SNAPSHOT).
    GetSnapshot {
        from_height: u64,
        limit: u32,
    },
    Snapshot {
        height: u64,
        tip: Hash,
        genesis: Hash,
        from_height: u64,
        blocks: Vec<Block>,
    },
    /// Header-only catch-up (Build/27 N6) — lighter than GetBlocks.
    GetHeaders {
        from_height: u64,
        limit: u32,
    },
    Headers {
        from_height: u64,
        headers: Vec<BlockHeader>,
    },
    /// Genesis-bound economic finality vote (Build/36 F2).
    FinalityAttest(FinalityAttestation),
    /// Soft slash freeze gossip (Build/27 N5) — settle still moves UTXOs on-chain.
    SlashMark {
        address: String,
        /// Optional pending settle txid (hex).
        txid: String,
        height: u64,
        stake_atomic: u64,
        peer_id: String,
    },
    /// Advertised node services (Build/27 N6) — e.g. archive, snapshot, ai_routing.
    /// Separate from Hello so mixed fleets keep Hello wire-compatible.
    ServiceAds {
        services: Vec<String>,
    },
    /// AI job assignment / broadcast (Build/10, Build/15). Payload is opaque JSON bytes.
    AiJob {
        job_id: String,
        kind: String,
        /// Blake3 commitment of job input.
        input_commitment: Hash,
        /// Optional worker hint (mesh address hex); empty = any.
        worker_hint: String,
        payload: Vec<u8>,
    },
    /// AI job result / receipt gossip.
    AiResult {
        job_id: String,
        worker: String,
        output_hash: Hash,
        latency_ms: u64,
        weight: u64,
        /// Serialized [`mesh_types::AiJobReceipt`] when available.
        receipt: Vec<u8>,
    },
    Ping,
    Pong,
}

impl NetMsg {
    /// Preferred gossip topic(s) for this message (publish to all returned).
    pub fn gossip_topics(&self) -> &'static [&'static str] {
        match self {
            NetMsg::Block(_)
            | NetMsg::Tx(_)
            | NetMsg::SlashMark { .. }
            | NetMsg::FinalityAttest(_)
            | NetMsg::ServiceAds { .. } => {
                &[MESH_GOSSIP_CHAIN, MESH_GOSSIP_TOPIC]
            }
            NetMsg::AiJob { .. } | NetMsg::AiResult { .. } => &[MESH_GOSSIP_AI, MESH_GOSSIP_TOPIC],
            _ => &[MESH_GOSSIP_TOPIC],
        }
    }
}
