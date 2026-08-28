use libp2p::gossipsub::IdentTopic;
use libp2p::identity::Keypair;
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::swarm::NetworkBehaviour;
use libp2p::{gossipsub, identify, ping, PeerId};

use crate::codec::{protocol_name, MeshCodec};
use crate::protocol::{MESH_GOSSIP_AI, MESH_GOSSIP_CHAIN, MESH_GOSSIP_TOPIC};

#[derive(NetworkBehaviour)]
pub struct MeshBehaviour {
    pub request_response: request_response::Behaviour<MeshCodec>,
    pub gossipsub: gossipsub::Behaviour,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
}

impl MeshBehaviour {
    pub fn new(keypair: &Keypair) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let peer_id = keypair.public().to_peer_id();

        let request_response = request_response::Behaviour::with_codec(
            MeshCodec,
            [(protocol_name(), ProtocolSupport::Full)],
            request_response::Config::default(),
        );

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .validation_mode(gossipsub::ValidationMode::Strict)
            .build()
            .map_err(|e| std::io::Error::other(e))?;

        let mut gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(keypair.clone()),
            gossipsub_config,
        )?;
        // Subscribe to legacy + split topics so mixed fleets interop (Build/10 N8).
        gossipsub.subscribe(&IdentTopic::new(MESH_GOSSIP_TOPIC))?;
        gossipsub.subscribe(&IdentTopic::new(MESH_GOSSIP_CHAIN))?;
        gossipsub.subscribe(&IdentTopic::new(MESH_GOSSIP_AI))?;

        let identify = identify::Behaviour::new(identify::Config::new(
            "/monkeymesh/1.0.0".into(),
            keypair.public(),
        ));

        let ping = ping::Behaviour::default();

        let _ = peer_id; // used implicitly by swarm
        Ok(Self {
            request_response,
            gossipsub,
            identify,
            ping,
        })
    }

    #[allow(dead_code)] // retained for callers / older topic helpers
    pub fn topic() -> IdentTopic {
        IdentTopic::new(MESH_GOSSIP_TOPIC)
    }

    pub fn topic_named(name: &str) -> IdentTopic {
        IdentTopic::new(name)
    }

    pub fn add_peer(&mut self, peer: PeerId) {
        self.gossipsub.add_explicit_peer(&peer);
    }
}
