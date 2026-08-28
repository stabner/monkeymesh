//! MonkeyMesh P2P: libp2p discovery + QUIC transport.
//!
//! Spec (`Build/10_NETWORK_PROTOCOL.md`): QUIC transport, libp2p discovery,
//! messages HELLO / PEER_LIST / TX / BLOCK / …

mod behaviour;
mod codec;
mod node;
mod protocol;

pub use node::{
    load_or_create_identity, parse_listen_addr, parse_seed_addr, run_node, socket_to_quic_multiaddr,
    InboundAiJob, NetworkHandle, NodeConfig, RelayEvent, SharedChain,
};
pub use protocol::{NetMsg, PROTOCOL_VERSION, MESH_GOSSIP_TOPIC, MESH_RR_PROTOCOL};
