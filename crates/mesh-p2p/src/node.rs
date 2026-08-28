use std::collections::{HashMap, HashSet};
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use libp2p::multiaddr::Protocol;
use libp2p::request_response::{self, OutboundRequestId};
use libp2p::swarm::{DialError, SwarmEvent};
use libp2p::{gossipsub, identity, Multiaddr, PeerId, Swarm};
use mesh_chain::{Chain, FinalityAttestation};
use mesh_types::{Address, AiJobReceipt, Block, Hash, NodeServiceKind, Transaction};
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{info, warn};

use crate::behaviour::{MeshBehaviour, MeshBehaviourEvent};
use crate::protocol::{NetMsg, PROTOCOL_VERSION};

pub type SharedChain = Arc<Mutex<Chain>>;

#[derive(Clone, Debug)]
pub struct NodeConfig {
    /// QUIC listen multiaddr, e.g. `/ip4/127.0.0.1/udp/39001/quic-v1`
    pub listen: Multiaddr,
    /// Seed multiaddrs to dial (with or without `/p2p/<peerid>`).
    pub seeds: Vec<Multiaddr>,
    pub dial_interval_secs: u64,
    /// Path to persist libp2p identity (ed25519 protobuf).
    pub identity_path: PathBuf,
}

/// Events relayed from local mining/wallet/AI into gossipsub.
#[derive(Clone, Debug)]
pub enum RelayEvent {
    Block(Block),
    Tx(Transaction),
    PeerList(Vec<String>),
    SlashMark {
        address: String,
        txid: String,
        height: u64,
        stake_atomic: u64,
        peer_id: String,
    },
    FinalityAttest(FinalityAttestation),
    AiJob {
        job_id: String,
        kind: String,
        input_commitment: Hash,
        worker_hint: String,
        payload: Vec<u8>,
    },
    AiResult {
        job_id: String,
        worker: String,
        output_hash: Hash,
        latency_ms: u64,
        weight: u64,
        receipt: Vec<u8>,
    },
}

/// Inbound AI job (for local JobQueue mirror). Receipts are applied in-swarm.
#[derive(Clone, Debug)]
pub struct InboundAiJob {
    pub job_id: String,
    pub kind: String,
    pub input_commitment: Hash,
    pub worker_hint: String,
    pub payload: Vec<u8>,
}

#[derive(Clone)]
pub struct NetworkHandle {
    relay: broadcast::Sender<RelayEvent>,
    cmd_tx: mpsc::UnboundedSender<NetCmd>,
    inbound_ai: broadcast::Sender<InboundAiJob>,
    peer_count: Arc<AtomicUsize>,
    /// libp2p ping RTT samples (peer → ms). Build/27 B8.
    peer_rtts: Arc<StdMutex<HashMap<PeerId, u64>>>,
    pub local_peer_id: PeerId,
}

enum NetCmd {
    Dial(Multiaddr),
}

impl NetworkHandle {
    pub fn announce_block(&self, block: Block) {
        let _ = self.relay.send(RelayEvent::Block(block));
    }

    pub fn announce_tx(&self, tx: Transaction) {
        let _ = self.relay.send(RelayEvent::Tx(tx));
    }

    pub fn announce_slash_mark(
        &self,
        address: String,
        txid: String,
        height: u64,
        stake_atomic: u64,
        peer_id: String,
    ) {
        let _ = self.relay.send(RelayEvent::SlashMark {
            address,
            txid,
            height,
            stake_atomic,
            peer_id,
        });
    }

    pub fn announce_finality_attest(&self, att: FinalityAttestation) {
        let _ = self.relay.send(RelayEvent::FinalityAttest(att));
    }

    pub fn announce_peer_list(&self, peers: Vec<String>) {
        let _ = self.relay.send(RelayEvent::PeerList(peers));
    }

    pub fn announce_ai_job(
        &self,
        job_id: String,
        kind: String,
        input_commitment: Hash,
        worker_hint: String,
        payload: Vec<u8>,
    ) {
        let _ = self.relay.send(RelayEvent::AiJob {
            job_id,
            kind,
            input_commitment,
            worker_hint,
            payload,
        });
    }

    pub fn announce_ai_result(
        &self,
        job_id: String,
        worker: String,
        output_hash: Hash,
        latency_ms: u64,
        weight: u64,
        receipt: Vec<u8>,
    ) {
        let _ = self.relay.send(RelayEvent::AiResult {
            job_id,
            worker,
            output_hash,
            latency_ms,
            weight,
            receipt,
        });
    }

    pub fn dial(&self, addr: Multiaddr) {
        let _ = self.cmd_tx.send(NetCmd::Dial(addr));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RelayEvent> {
        self.relay.subscribe()
    }

    pub fn subscribe_ai_jobs(&self) -> broadcast::Receiver<InboundAiJob> {
        self.inbound_ai.subscribe()
    }

    pub fn peer_count(&self) -> usize {
        self.peer_count.load(Ordering::Relaxed)
    }

    /// Connected peer RTT samples from libp2p ping (`peer_id`, ms), lowest first.
    pub fn peer_rtt_snapshot(&self) -> Vec<(String, u64)> {
        let g = self
            .peer_rtts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<_> = g
            .iter()
            .map(|(p, ms)| (p.to_string(), *ms))
            .collect();
        v.sort_by_key(|(_, ms)| *ms);
        v
    }

    /// Median peer RTT in milliseconds (None if no samples yet).
    pub fn median_peer_rtt_ms(&self) -> Option<u64> {
        let snap = self.peer_rtt_snapshot();
        if snap.is_empty() {
            None
        } else {
            Some(snap[snap.len() / 2].1)
        }
    }
}

/// Start libp2p QUIC swarm. Returns a handle for announcing local blocks/txs/AI.
pub async fn run_node(chain: SharedChain, cfg: NodeConfig) -> Result<NetworkHandle> {
    let id_keys = load_or_create_identity(&cfg.identity_path)?;
    let local_peer_id = id_keys.public().to_peer_id();
    info!(%local_peer_id, "libp2p identity");

    let behaviour = MeshBehaviour::new(&id_keys).map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_quic()
        .with_behaviour(|_| behaviour)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    swarm.listen_on(cfg.listen.clone())
        .with_context(|| format!("listen on {}", cfg.listen))?;

    let (relay_tx, mut relay_rx) = broadcast::channel::<RelayEvent>(256);
    let (inbound_ai_tx, _) = broadcast::channel::<InboundAiJob>(2_048);
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<NetCmd>();
    let peer_count = Arc::new(AtomicUsize::new(0));
    let peer_rtts: Arc<StdMutex<HashMap<PeerId, u64>>> =
        Arc::new(StdMutex::new(HashMap::new()));

    let handle = NetworkHandle {
        relay: relay_tx.clone(),
        cmd_tx,
        inbound_ai: inbound_ai_tx.clone(),
        peer_count: peer_count.clone(),
        peer_rtts: peer_rtts.clone(),
        local_peer_id,
    };

    let listen_addr = cfg.listen.clone();
    let seeds = cfg.seeds.clone();
    let dial_interval = Duration::from_secs(cfg.dial_interval_secs.max(3));

    tokio::spawn(async move {
        let mut connected: HashSet<PeerId> = HashSet::new();
        let mut pending_hello: HashSet<PeerId> = HashSet::new();
        let mut hello_reqs: HashSet<OutboundRequestId> = HashSet::new();
        let mut learned: HashSet<Multiaddr> = HashSet::new();
        let mut peer_listen: HashMap<PeerId, String> = HashMap::new();
        let mut peer_services: HashMap<PeerId, Vec<String>> = HashMap::new();
        let mut dial_tick = tokio::time::interval(dial_interval);
        dial_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut peerlist_tick = tokio::time::interval(Duration::from_secs(30));
        peerlist_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        for addr in &seeds {
            learned.insert(addr.clone());
            dial_addr(&mut swarm, addr, &listen_addr);
        }

        loop {
            tokio::select! {
                event = swarm.select_next_some() => {
                    if let Err(e) = handle_swarm_event(
                        &mut swarm,
                        &chain,
                        &relay_tx,
                        &inbound_ai_tx,
                        &listen_addr,
                        event,
                        &mut connected,
                        &mut pending_hello,
                        &mut hello_reqs,
                        &mut learned,
                        &mut peer_listen,
                        &mut peer_services,
                        &peer_count,
                        &peer_rtts,
                    ).await {
                        warn!(error = %e, "swarm handler error");
                    }
                }
                evt = relay_rx.recv() => {
                    match evt {
                        Ok(RelayEvent::Block(b)) => {
                            announce_to_network(&mut swarm, &connected, NetMsg::Block(b));
                        }
                        Ok(RelayEvent::Tx(t)) => {
                            announce_to_network(&mut swarm, &connected, NetMsg::Tx(t));
                        }
                        Ok(RelayEvent::SlashMark {
                            address,
                            txid,
                            height,
                            stake_atomic,
                            peer_id,
                        }) => {
                            announce_to_network(
                                &mut swarm,
                                &connected,
                                NetMsg::SlashMark {
                                    address,
                                    txid,
                                    height,
                                    stake_atomic,
                                    peer_id,
                                },
                            );
                        }
                        Ok(RelayEvent::FinalityAttest(att)) => {
                            announce_to_network(
                                &mut swarm,
                                &connected,
                                NetMsg::FinalityAttest(att),
                            );
                        }
                        Ok(RelayEvent::PeerList(peers)) => {
                            announce_to_network(
                                &mut swarm,
                                &connected,
                                NetMsg::PeerList { peers },
                            );
                        }
                        Ok(RelayEvent::AiJob {
                            job_id,
                            kind,
                            input_commitment,
                            worker_hint,
                            payload,
                        }) => {
                            announce_to_network(
                                &mut swarm,
                                &connected,
                                NetMsg::AiJob {
                                    job_id,
                                    kind,
                                    input_commitment,
                                    worker_hint,
                                    payload,
                                },
                            );
                        }
                        Ok(RelayEvent::AiResult {
                            job_id,
                            worker,
                            output_hash,
                            latency_ms,
                            weight,
                            receipt,
                        }) => {
                            announce_to_network(
                                &mut swarm,
                                &connected,
                                NetMsg::AiResult {
                                    job_id,
                                    worker,
                                    output_hash,
                                    latency_ms,
                                    weight,
                                    receipt,
                                },
                            );
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(NetCmd::Dial(addr)) => {
                            if !multiaddr_is_self(&addr, &listen_addr) {
                                learned.insert(addr.clone());
                                dial_addr(&mut swarm, &addr, &listen_addr);
                            }
                        }
                        None => break,
                    }
                }
                _ = dial_tick.tick() => {
                    for addr in learned.iter() {
                        dial_addr(&mut swarm, addr, &listen_addr);
                    }
                }
                _ = peerlist_tick.tick() => {
                    if connected.is_empty() {
                        continue;
                    }
                    let mut peers: Vec<String> = Vec::new();
                    peers.push(advertised_listen_addr(&listen_addr).to_string());
                    for a in peer_listen.values() {
                        if !a.is_empty() {
                            peers.push(a.clone());
                        }
                    }
                    for a in &learned {
                        peers.push(a.to_string());
                    }
                    peers.sort();
                    peers.dedup();
                    announce_to_network(
                        &mut swarm,
                        &connected,
                        NetMsg::PeerList { peers },
                    );
                    let ads = make_service_ads(&chain).await;
                    announce_to_network(&mut swarm, &connected, ads);
                }
            }
        }
    });

    Ok(handle)
}

fn dial_addr(swarm: &mut Swarm<MeshBehaviour>, addr: &Multiaddr, our_listen: &Multiaddr) {
    if multiaddr_undialable(addr) || multiaddr_is_self(addr, our_listen) {
        tracing::debug!(%addr, "skip undialable/self dial");
        return;
    }
    match swarm.dial(addr.clone()) {
        Ok(()) => tracing::debug!(%addr, "dialing"),
        Err(DialError::NoAddresses) => warn!(%addr, "dial: no addresses"),
        Err(DialError::Aborted) => {}
        Err(DialError::WrongPeerId { obtained, .. }) => {
            warn!(%addr, %obtained, "dial wrong peer id")
        }
        Err(DialError::LocalPeerId { .. }) => {}
        Err(DialError::Denied { cause }) => warn!(%addr, %cause, "dial denied"),
        Err(DialError::DialPeerConditionFalse(_)) => {}
        Err(e) => tracing::debug!(%addr, error = %e, "dial skipped/failed"),
    }
}

fn learn_and_dial(
    swarm: &mut Swarm<MeshBehaviour>,
    learned: &mut HashSet<Multiaddr>,
    addr_str: &str,
    our_listen: &Multiaddr,
) {
    let Ok(addr) = parse_listen_addr(addr_str) else {
        return;
    };
    if multiaddr_undialable(&addr) || multiaddr_is_self(&addr, our_listen) {
        tracing::debug!(%addr, "skip undialable/self learned addr");
        return;
    }
    if learned.insert(addr.clone()) {
        tracing::debug!(%addr, "learned peer addr");
    }
    dial_addr(swarm, &addr, our_listen);
}

/// Unspecified / bind-any addresses are not reachable peers.
fn multiaddr_undialable(addr: &Multiaddr) -> bool {
    for p in addr.iter() {
        match p {
            Protocol::Ip4(ip) if ip.is_unspecified() => return true,
            Protocol::Ip6(ip) if ip.is_unspecified() => return true,
            _ => {}
        }
    }
    false
}

fn multiaddr_udp_port(addr: &Multiaddr) -> Option<u16> {
    addr.iter().find_map(|p| match p {
        Protocol::Udp(p) => Some(p),
        _ => None,
    })
}

/// Same UDP port as our listen ⇒ this process (0.0.0.0 bind + MESH_ADVERTISE_HOST).
fn multiaddr_is_self(addr: &Multiaddr, our_listen: &Multiaddr) -> bool {
    match (multiaddr_udp_port(addr), multiaddr_udp_port(our_listen)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Mine-only edges proxy AI to seed — ignore AI gossip to keep RPC/templates responsive.
fn edge_skips_ai_gossip() -> bool {
    let edge = std::env::var("MESH_EDGE_MODE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !edge {
        return false;
    }
    // MESH_EDGE_AI_LOCAL=0 (or unset with upstream) → skip. Local/hybrid boards keep gossip.
    std::env::var("MESH_EDGE_AI_LOCAL")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
}

/// Rewrite Hello listen to MESH_ADVERTISE_HOST while keeping the UDP port.
fn advertised_listen_addr(listen: &Multiaddr) -> Multiaddr {
    let host = match std::env::var("MESH_ADVERTISE_HOST") {
        Ok(h) => {
            let t = h.trim().to_string();
            if t.is_empty() {
                return listen.clone();
            }
            t
        }
        Err(_) => return listen.clone(),
    };
    let mut port: Option<u16> = None;
    for p in listen.iter() {
        if let Protocol::Udp(u) = p {
            port = Some(u);
        }
    }
    let Some(port) = port else {
        return listen.clone();
    };
    let sock = format!("{host}:{port}");
    parse_listen_addr(&sock).unwrap_or_else(|_| listen.clone())
}

fn publish_gossip(swarm: &mut Swarm<MeshBehaviour>, msg: &NetMsg) -> bool {
    let Ok(data) = bincode::serialize(msg) else {
        return false;
    };
    let mut any_ok = false;
    for topic in msg.gossip_topics() {
        match swarm
            .behaviour_mut()
            .gossipsub
            .publish(MeshBehaviour::topic_named(topic), data.clone())
        {
            Ok(_) => any_ok = true,
            Err(e) => {
                tracing::debug!(topic, error = %e, "gossip publish failed");
            }
        }
    }
    if !any_ok {
        tracing::info!("gossip publish failed on all topics (will still RR peers)");
    }
    any_ok
}

fn announce_to_network(
    swarm: &mut Swarm<MeshBehaviour>,
    connected: &HashSet<PeerId>,
    msg: NetMsg,
) {
    let kind = match &msg {
        NetMsg::Block(b) => format!("block h={}", b.header.height),
        NetMsg::Tx(t) => format!("tx {}", t.txid()),
        NetMsg::SlashMark { address, height, .. } => {
            format!("slashmark {address} h={height}")
        }
        NetMsg::FinalityAttest(att) => {
            format!("finality h={} {}", att.height, att.block_hash)
        }
        NetMsg::PeerList { peers } => format!("peerlist n={}", peers.len()),
        NetMsg::AiJob { job_id, kind, .. } => format!("ai_job {kind} {job_id}"),
        NetMsg::AiResult { job_id, .. } => format!("ai_result {job_id}"),
        _ => "msg".into(),
    };
    let gossip_ok = publish_gossip(swarm, &msg);
    // AI traffic is high-volume — gossip only. RR-to-all-peers doubled load and
    // stalled mine edges importing receipts under the chain lock.
    let skip_rr = matches!(
        &msg,
        NetMsg::AiJob { .. } | NetMsg::AiResult { .. } | NetMsg::PeerList { .. }
    );
    let mut rr = 0usize;
    if !skip_rr {
        for peer in connected.iter().copied() {
            send_rr(swarm, peer, msg.clone());
            rr += 1;
        }
    }
    tracing::debug!(%kind, gossip_ok, rr_peers = rr, "announced to network");
}

fn send_rr(swarm: &mut Swarm<MeshBehaviour>, peer: PeerId, msg: NetMsg) -> OutboundRequestId {
    swarm
        .behaviour_mut()
        .request_response
        .send_request(&peer, msg)
}

async fn handle_swarm_event(
    swarm: &mut Swarm<MeshBehaviour>,
    chain: &SharedChain,
    relay: &broadcast::Sender<RelayEvent>,
    inbound_ai: &broadcast::Sender<InboundAiJob>,
    listen_addr: &Multiaddr,
    event: SwarmEvent<MeshBehaviourEvent>,
    connected: &mut HashSet<PeerId>,
    pending_hello: &mut HashSet<PeerId>,
    hello_reqs: &mut HashSet<OutboundRequestId>,
    learned: &mut HashSet<Multiaddr>,
    peer_listen: &mut HashMap<PeerId, String>,
    peer_services: &mut HashMap<PeerId, Vec<String>>,
    peer_count: &AtomicUsize,
    peer_rtts: &StdMutex<HashMap<PeerId, u64>>,
) -> Result<()> {
    match event {
        SwarmEvent::NewListenAddr { address, .. } => {
            let with_p2p = address
                .clone()
                .with_p2p(*swarm.local_peer_id())
                .unwrap_or(address.clone());
            info!(%with_p2p, "p2p listening (QUIC)");
        }
        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
            if connected.insert(peer_id) {
                info!(%peer_id, "peer connected");
                peer_count.store(connected.len(), Ordering::Relaxed);
                swarm.behaviour_mut().add_peer(peer_id);
                if pending_hello.insert(peer_id) {
                    let hello = make_hello(chain, listen_addr).await;
                    let id = send_rr(swarm, peer_id, hello);
                    hello_reqs.insert(id);
                }
            }
        }
        SwarmEvent::ConnectionClosed { peer_id, .. } => {
            connected.remove(&peer_id);
            pending_hello.remove(&peer_id);
            peer_listen.remove(&peer_id);
            peer_services.remove(&peer_id);
            if let Ok(mut g) = peer_rtts.lock() {
                g.remove(&peer_id);
            }
            peer_count.store(connected.len(), Ordering::Relaxed);
            info!(%peer_id, "peer disconnected");
        }
        SwarmEvent::Behaviour(MeshBehaviourEvent::RequestResponse(ev)) => {
            handle_rr(
                swarm,
                chain,
                relay,
                inbound_ai,
                listen_addr,
                ev,
                hello_reqs,
                learned,
                peer_listen,
                peer_services,
                connected,
                peer_rtts,
            )
            .await?;
        }
        SwarmEvent::Behaviour(MeshBehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source,
            message,
            ..
        })) => match bincode::deserialize::<NetMsg>(&message.data) {
            Ok(msg) => {
                handle_net_msg(
                    swarm,
                    chain,
                    relay,
                    inbound_ai,
                    learned,
                    peer_listen,
                    peer_services,
                    Some(propagation_source),
                    msg,
                    false,
                    listen_addr,
                )
                .await?;
            }
            Err(e) => warn!(error = %e, "bad gossip payload"),
        },
        SwarmEvent::Behaviour(MeshBehaviourEvent::Identify(ev)) => {
            tracing::debug!(?ev, "identify");
        }
        SwarmEvent::Behaviour(MeshBehaviourEvent::Ping(ev)) => {
            match ev.result {
                Ok(rtt) => {
                    let ms = if rtt.is_zero() {
                        0u64
                    } else {
                        rtt.as_millis().max(1).min(u128::from(u64::MAX)) as u64
                    };
                    let median = {
                        let mut g = peer_rtts.lock().unwrap_or_else(|e| e.into_inner());
                        g.insert(ev.peer, ms);
                        median_from_rtt_map(&g)
                    };
                    let factor = mesh_chain::rtt_factor_milli(median);
                    {
                        let mut c = chain.lock().await;
                        c.set_relay_rtt_factor_milli(factor);
                    }
                    tracing::debug!(peer = %ev.peer, rtt_ms = ms, "ping ok");
                }
                Err(e) => {
                    if let Ok(mut g) = peer_rtts.lock() {
                        g.remove(&ev.peer);
                    }
                    tracing::debug!(peer = %ev.peer, error = %e, "ping fail");
                }
            }
        }
        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
            tracing::debug!(?peer_id, %error, "outgoing connection error");
        }
        SwarmEvent::IncomingConnectionError { error, .. } => {
            tracing::debug!(%error, "incoming connection error");
        }
        _ => {}
    }
    Ok(())
}

fn median_from_rtt_map(map: &HashMap<PeerId, u64>) -> Option<u64> {
    if map.is_empty() {
        return None;
    }
    let mut v: Vec<u64> = map.values().copied().collect();
    v.sort_unstable();
    Some(v[v.len() / 2])
}

async fn handle_rr(
    swarm: &mut Swarm<MeshBehaviour>,
    chain: &SharedChain,
    relay: &broadcast::Sender<RelayEvent>,
    inbound_ai: &broadcast::Sender<InboundAiJob>,
    listen_addr: &Multiaddr,
    ev: request_response::Event<NetMsg, NetMsg>,
    hello_reqs: &mut HashSet<OutboundRequestId>,
    learned: &mut HashSet<Multiaddr>,
    peer_listen: &mut HashMap<PeerId, String>,
    peer_services: &mut HashMap<PeerId, Vec<String>>,
    connected: &HashSet<PeerId>,
    peer_rtts: &StdMutex<HashMap<PeerId, u64>>,
) -> Result<()> {
    match ev {
        request_response::Event::Message { peer, message, .. } => match message {
            request_response::Message::Request {
                request,
                channel,
                ..
            } => {
                let follow_ups = follow_ups_after_hello(&request, chain).await?;
                let response = handle_request(
                    swarm,
                    chain,
                    relay,
                    inbound_ai,
                    peer,
                    request,
                    learned,
                    peer_listen,
                    peer_services,
                    listen_addr,
                )
                .await?;
                let _ = swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(channel, response);
                let rtts = peer_rtts.lock().unwrap_or_else(|e| e.into_inner()).clone();
                for msg in follow_ups {
                    let target = pick_sync_peer(peer, connected, peer_services, &rtts, &msg);
                    send_rr(swarm, target, msg);
                }
            }
            request_response::Message::Response {
                request_id,
                response,
            } => {
                let was_hello = hello_reqs.remove(&request_id);
                handle_response(
                    swarm,
                    chain,
                    relay,
                    listen_addr,
                    peer,
                    response,
                    was_hello,
                    peer_services,
                    connected,
                    peer_rtts,
                )
                .await?;
            }
        },
        request_response::Event::OutboundFailure { peer, error, .. } => {
            warn!(%peer, %error, "rr outbound failure");
        }
        request_response::Event::InboundFailure { peer, error, .. } => {
            warn!(%peer, %error, "rr inbound failure");
        }
        request_response::Event::ResponseSent { .. } => {}
    }
    Ok(())
}

fn peer_has_archive(services: Option<&Vec<String>>) -> bool {
    services
        .map(|s| s.iter().any(|x| x == "archive" || x == "snapshot"))
        .unwrap_or(false)
}

fn wants_archive_peer(msg: &NetMsg) -> bool {
    matches!(
        msg,
        NetMsg::GetHeaders { .. } | NetMsg::GetSnapshot { .. }
    )
}

fn pick_sync_peer(
    preferred: PeerId,
    connected: &HashSet<PeerId>,
    peer_services: &HashMap<PeerId, Vec<String>>,
    peer_rtts: &HashMap<PeerId, u64>,
    msg: &NetMsg,
) -> PeerId {
    pick_sync_peer_excluding(preferred, connected, peer_services, peer_rtts, msg, None)
}

fn pick_sync_peer_excluding(
    preferred: PeerId,
    connected: &HashSet<PeerId>,
    peer_services: &HashMap<PeerId, Vec<String>>,
    peer_rtts: &HashMap<PeerId, u64>,
    msg: &NetMsg,
    exclude: Option<PeerId>,
) -> PeerId {
    if !wants_archive_peer(msg) {
        return preferred;
    }
    let try_archive = |skip: Option<PeerId>| -> Option<PeerId> {
        let mut best: Option<(PeerId, u64)> = None;
        for p in connected {
            if Some(*p) == skip {
                continue;
            }
            if peer_has_archive(peer_services.get(p)) {
                let rtt = peer_rtts.get(p).copied().unwrap_or(u64::MAX);
                match best {
                    None => best = Some((*p, rtt)),
                    Some((_, br)) if rtt < br => best = Some((*p, rtt)),
                    _ => {}
                }
            }
        }
        best.map(|(p, _)| p)
    };
    match peer_services.get(&preferred) {
        Some(s) if peer_has_archive(Some(s)) && exclude != Some(preferred) => preferred,
        Some(_) => {
            if let Some(p) = try_archive(exclude.or(Some(preferred))) {
                info!(%p, preferred = %preferred, "preferring lowest-RTT archive/snapshot peer for catch-up");
                return p;
            }
            preferred
        }
        None => {
            // Ads unknown — still prefer any peer that already advertised archive.
            if let Some(p) = try_archive(exclude) {
                if p != preferred {
                    info!(%p, preferred = %preferred, "preferring known archive peer (preferred ads unknown)");
                    return p;
                }
            }
            preferred
        }
    }
}

async fn local_services(chain: &SharedChain) -> Vec<String> {
    let c = chain.lock().await;
    let height = c.height();
    let blocks = c.store().len();
    let full_history = !c.store().is_pruned() && blocks > 0 && blocks as u64 == height.saturating_add(1);
    let mut services = vec!["tx_relay".into(), "block_relay".into()];
    if full_history {
        services.push("archive".into());
        services.push("snapshot".into());
    }
    let edge = std::env::var("MESH_EDGE_MODE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !edge || std::env::var("MESH_AI_UPSTREAM").is_ok() {
        services.push("ai_routing".into());
    }
    services
}

async fn make_service_ads(chain: &SharedChain) -> NetMsg {
    NetMsg::ServiceAds {
        services: local_services(chain).await,
    }
}

async fn make_hello(chain: &SharedChain, listen_addr: &Multiaddr) -> NetMsg {
    let advertised = advertised_listen_addr(listen_addr);
    let c = chain.lock().await;
    NetMsg::Hello {
        version: PROTOCOL_VERSION,
        height: c.height(),
        tip: c.tip_hash(),
        genesis: c.genesis_hash(),
        listen_addr: advertised.to_string(),
    }
}

async fn follow_ups_after_hello(request: &NetMsg, chain: &SharedChain) -> Result<Vec<NetMsg>> {
    let NetMsg::Hello {
        version,
        height,
        tip,
        genesis,
        ..
    } = request
    else {
        return Ok(Vec::new());
    };
    if *version != PROTOCOL_VERSION {
        return Ok(Vec::new());
    }
    let (our_genesis, our_height, our_tip, pruned) = {
        let c = chain.lock().await;
        (c.genesis_hash(), c.height(), c.tip_hash(), c.store().is_pruned())
    };
    if *genesis != our_genesis {
        return Ok(Vec::new());
    }
    let mut out = vec![make_service_ads(chain).await];
    if *height > our_height {
        let gap = height.saturating_sub(our_height);
        // Prefer headers/snapshot when gap is large or we are pruned (need archive peer).
        // Large IBD: skip header ping-pong — pull max snapshot batches directly.
        if gap > 64 || (pruned && gap > 16) {
            out.push(NetMsg::GetSnapshot {
                from_height: our_height.saturating_add(1),
                limit: 500,
            });
        } else {
            out.push(NetMsg::GetBlocks {
                from_height: our_height.saturating_add(1),
                limit: 500,
            });
        }
    } else if *height == our_height && *tip != our_tip {
        // Same height, different tip — pull their block for depth-1 / reorg.
        out.push(NetMsg::GetBlocks {
            from_height: *height,
            limit: 1,
        });
    }
    Ok(out)
}

async fn handle_request(
    swarm: &mut Swarm<MeshBehaviour>,
    chain: &SharedChain,
    relay: &broadcast::Sender<RelayEvent>,
    inbound_ai: &broadcast::Sender<InboundAiJob>,
    peer: PeerId,
    request: NetMsg,
    learned: &mut HashSet<Multiaddr>,
    peer_listen: &mut HashMap<PeerId, String>,
    peer_services: &mut HashMap<PeerId, Vec<String>>,
    our_listen: &Multiaddr,
) -> Result<NetMsg> {
    match request {
        NetMsg::Hello {
            version,
            height,
            tip,
            genesis,
            listen_addr,
        } => {
            if version != PROTOCOL_VERSION {
                bail!("peer protocol version {version}");
            }
            let (our_genesis, our_height, our_tip) = {
                let c = chain.lock().await;
                (c.genesis_hash(), c.height(), c.tip_hash())
            };
            if genesis != our_genesis {
                bail!("genesis mismatch peer={genesis} local={our_genesis}");
            }
            info!(
                %peer,
                peer_height = height,
                peer_tip = %tip,
                peer_listen = %listen_addr,
                "hello received"
            );
            if !listen_addr.is_empty() {
                peer_listen.insert(peer, listen_addr.clone());
                learn_and_dial(swarm, learned, &listen_addr, our_listen);
            }
            Ok(NetMsg::HelloAck {
                version: PROTOCOL_VERSION,
                height: our_height,
                tip: our_tip,
                genesis: our_genesis,
            })
        }
        NetMsg::ServiceAds { services } => {
            info!(%peer, ?services, "service ads received");
            peer_services.insert(peer, services);
            Ok(NetMsg::Pong)
        }
        NetMsg::GetBlocks { from_height, limit } => {
            let limit = limit.min(500);
            let blocks = {
                let mut c = chain.lock().await;
                let blocks = c.blocks_from(from_height, limit);
                if !blocks.is_empty() {
                    let w = (blocks.len() as u64).min(8).max(1);
                    let _ = c.credit_local_service(NodeServiceKind::BlockRelay, w);
                }
                blocks
            };
            info!(%peer, from_height, n = blocks.len(), "serving getblocks");
            Ok(NetMsg::Blocks(blocks))
        }
        NetMsg::GetSnapshot { from_height, limit } => {
            let limit = limit.min(500);
            let (height, tip, genesis, blocks) = {
                let mut c = chain.lock().await;
                let blocks = c.blocks_from(from_height, limit);
                if !blocks.is_empty() {
                    let w = (blocks.len() as u64).min(8).max(1);
                    let _ = c.credit_local_service(NodeServiceKind::Snapshot, w);
                }
                (c.height(), c.tip_hash(), c.genesis_hash(), blocks)
            };
            info!(
                %peer,
                from_height,
                n = blocks.len(),
                tip_height = height,
                "serving snapshot"
            );
            Ok(NetMsg::Snapshot {
                height,
                tip,
                genesis,
                from_height,
                blocks,
            })
        }
        NetMsg::GetHeaders { from_height, limit } => {
            let limit = limit.min(2_000);
            let headers = {
                let mut c = chain.lock().await;
                let headers: Vec<_> = c
                    .blocks_from(from_height, limit)
                    .into_iter()
                    .map(|b| b.header)
                    .collect();
                if !headers.is_empty() {
                    let w = (headers.len() as u64 / 32).max(1).min(8);
                    let _ = c.credit_local_service(NodeServiceKind::Archive, w);
                }
                headers
            };
            info!(%peer, from_height, n = headers.len(), "serving getheaders");
            Ok(NetMsg::Headers {
                from_height,
                headers,
            })
        }
        NetMsg::Ping => Ok(NetMsg::Pong),
        other => {
            handle_net_msg(
                swarm,
                chain,
                relay,
                inbound_ai,
                learned,
                peer_listen,
                peer_services,
                Some(peer),
                other,
                true,
                our_listen,
            )
            .await?;
            Ok(NetMsg::Pong)
        }
    }
}

async fn handle_response(
    swarm: &mut Swarm<MeshBehaviour>,
    chain: &SharedChain,
    relay: &broadcast::Sender<RelayEvent>,
    listen_addr: &Multiaddr,
    peer: PeerId,
    response: NetMsg,
    was_hello: bool,
    peer_services: &mut HashMap<PeerId, Vec<String>>,
    connected: &HashSet<PeerId>,
    peer_rtts: &StdMutex<HashMap<PeerId, u64>>,
) -> Result<()> {
    match response {
        NetMsg::HelloAck {
            version,
            height,
            tip,
            genesis,
        } => {
            if version != PROTOCOL_VERSION {
                bail!("bad hello ack version");
            }
            let (our_genesis, our_height, pruned) = {
                let c = chain.lock().await;
                (c.genesis_hash(), c.height(), c.store().is_pruned())
            };
            if genesis != our_genesis {
                bail!("genesis mismatch on ack");
            }
            info!(%peer, peer_height = height, peer_tip = %tip, "hello ack");
            // Advertise our services so the peer can prefer archive hosts.
            send_rr(swarm, peer, make_service_ads(chain).await);
            if height > our_height {
                let gap = height.saturating_sub(our_height);
                let msg = if gap > 64 || (pruned && gap > 16) {
                    NetMsg::GetSnapshot {
                        from_height: our_height.saturating_add(1),
                        limit: 500,
                    }
                } else {
                    NetMsg::GetBlocks {
                        from_height: our_height.saturating_add(1),
                        limit: 500,
                    }
                };
                let rtts = peer_rtts.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let target = pick_sync_peer(peer, connected, peer_services, &rtts, &msg);
                send_rr(swarm, target, msg);
            } else if height < our_height {
                // We are ahead — push the missing blocks so a finder is not
                // the only node that ever announces the tip.
                let blocks = {
                    let c = chain.lock().await;
                    c.blocks_from(height.saturating_add(1), 32)
                };
                for b in blocks {
                    send_rr(swarm, peer, NetMsg::Block(b));
                }
            } else {
                let our_tip = {
                    let c = chain.lock().await;
                    c.tip_hash()
                };
                if tip != our_tip {
                    send_rr(
                        swarm,
                        peer,
                        NetMsg::GetBlocks {
                            from_height: height,
                            limit: 1,
                        },
                    );
                } else if was_hello {
                    let _ = listen_addr;
                }
            }
        }
        NetMsg::ServiceAds { services } => {
            info!(%peer, ?services, "service ads (response)");
            peer_services.insert(peer, services);
        }
        NetMsg::Headers {
            from_height,
            headers,
        } => {
            info!(%peer, from_height, n = headers.len(), "headers received");
            if headers.is_empty() {
                let msg = NetMsg::GetHeaders {
                    from_height,
                    limit: 512,
                };
                let rtts = peer_rtts.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let target = pick_sync_peer_excluding(
                    peer,
                    connected,
                    peer_services,
                    &rtts,
                    &msg,
                    Some(peer),
                );
                if target != peer {
                    info!(%target, failed = %peer, "empty headers — retry archive peer");
                    send_rr(swarm, target, msg);
                }
            } else if let Some(last) = headers.last() {
                let our_height = {
                    let c = chain.lock().await;
                    c.height()
                };
                if last.height > our_height {
                    let msg = NetMsg::GetSnapshot {
                        from_height: our_height.saturating_add(1),
                        limit: 500,
                    };
                    let rtts = peer_rtts.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    let target = pick_sync_peer(peer, connected, peer_services, &rtts, &msg);
                    send_rr(swarm, target, msg);
                }
            }
        }
        NetMsg::Blocks(blocks) => {
            let n = blocks.len();
            for block in blocks {
                import_block(chain, relay, block).await?;
            }
            if n >= 500 {
                // Full batch — keep streaming without Hello round-trip.
                let our_height = {
                    let c = chain.lock().await;
                    c.height()
                };
                let msg = NetMsg::GetBlocks {
                    from_height: our_height.saturating_add(1),
                    limit: 500,
                };
                let rtts = peer_rtts.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let target = pick_sync_peer(peer, connected, peer_services, &rtts, &msg);
                send_rr(swarm, target, msg);
            } else {
                let hello = make_hello(chain, listen_addr).await;
                send_rr(swarm, peer, hello);
            }
        }
        NetMsg::Snapshot {
            height,
            tip,
            genesis,
            from_height,
            blocks,
        } => {
            let our_genesis = {
                let c = chain.lock().await;
                c.genesis_hash()
            };
            if genesis != our_genesis {
                bail!("snapshot genesis mismatch");
            }
            info!(
                %peer,
                from_height,
                n = blocks.len(),
                peer_height = height,
                peer_tip = %tip,
                "snapshot received"
            );
            if blocks.is_empty() {
                let our_height = {
                    let c = chain.lock().await;
                    c.height()
                };
                if height > our_height {
                    let msg = NetMsg::GetSnapshot {
                        from_height: our_height.saturating_add(1).max(from_height),
                        limit: 500,
                    };
                    let rtts = peer_rtts.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    let target = pick_sync_peer_excluding(
                        peer,
                        connected,
                        peer_services,
                        &rtts,
                        &msg,
                        Some(peer),
                    );
                    if target != peer {
                        info!(%target, failed = %peer, "empty snapshot — retry archive peer");
                        send_rr(swarm, target, msg);
                    }
                }
            } else {
                for block in blocks {
                    import_block(chain, relay, block).await?;
                }
                let our_height = {
                    let c = chain.lock().await;
                    c.height()
                };
                if height > our_height {
                    let msg = NetMsg::GetSnapshot {
                        from_height: our_height.saturating_add(1),
                        limit: 500,
                    };
                    let rtts = peer_rtts.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    let target = pick_sync_peer(peer, connected, peer_services, &rtts, &msg);
                    send_rr(swarm, target, msg);
                } else {
                    let hello = make_hello(chain, listen_addr).await;
                    send_rr(swarm, peer, hello);
                }
            }
        }
        NetMsg::Pong => {}
        other => tracing::debug!(?other, %peer, "unexpected response"),
    }
    Ok(())
}

async fn handle_net_msg(
    swarm: &mut Swarm<MeshBehaviour>,
    chain: &SharedChain,
    relay: &broadcast::Sender<RelayEvent>,
    inbound_ai: &broadcast::Sender<InboundAiJob>,
    learned: &mut HashSet<Multiaddr>,
    peer_listen: &mut HashMap<PeerId, String>,
    peer_services: &mut HashMap<PeerId, Vec<String>>,
    from: Option<PeerId>,
    msg: NetMsg,
    _via_rr: bool,
    our_listen: &Multiaddr,
) -> Result<()> {
    match msg {
        NetMsg::Block(block) => {
            if let Some(from) = from {
                tracing::debug!(%from, height = block.header.height, "net block");
            }
            import_block(chain, relay, block).await?;
        }
        NetMsg::Tx(tx) => {
            if let Some(from) = from {
                tracing::debug!(%from, "net tx");
            }
            import_tx(chain, relay, tx).await?;
        }
        NetMsg::ServiceAds { services } => {
            if let Some(from) = from {
                info!(%from, ?services, "service ads (gossip)");
                peer_services.insert(from, services);
            }
        }
        NetMsg::SlashMark {
            address,
            txid,
            height,
            stake_atomic,
            peer_id,
        } => {
            if let Some(from) = from {
                tracing::debug!(%from, %address, height, "net slashmark");
            }
            import_slash_mark(chain, &address, height, stake_atomic, &peer_id, &txid).await?;
        }
        NetMsg::FinalityAttest(att) => {
            if let Some(from) = from {
                tracing::debug!(%from, height = att.height, "net finality");
            }
            import_finality_attest(chain, relay, att).await?;
        }
        NetMsg::PeerList { peers } => {
            for p in peers {
                learn_and_dial(swarm, learned, &p, our_listen);
            }
            let _ = peer_listen;
            let _ = peer_services;
        }
        NetMsg::AiJob {
            job_id,
            kind,
            input_commitment,
            worker_hint,
            payload,
        } => {
            if edge_skips_ai_gossip() {
                return Ok(());
            }
            let _ = inbound_ai.send(InboundAiJob {
                job_id,
                kind,
                input_commitment,
                worker_hint,
                payload,
            });
            let mut c = chain.lock().await;
            let _ = c.credit_local_service(NodeServiceKind::AiRouting, 2);
        }
        NetMsg::AiResult {
            job_id,
            worker: _,
            output_hash: _,
            latency_ms: _,
            weight: _,
            receipt,
        } => {
            if edge_skips_ai_gossip() {
                return Ok(());
            }
            import_ai_result(chain, &job_id, &receipt).await?;
        }
        _ => {}
    }
    Ok(())
}

async fn import_ai_result(chain: &SharedChain, job_id: &str, receipt_bytes: &[u8]) -> Result<()> {
    if receipt_bytes.is_empty() {
        return Ok(());
    }
    let receipt: AiJobReceipt = match bincode::deserialize(receipt_bytes) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, job_id, "bad ai receipt bytes");
            return Ok(());
        }
    };
    let mut c = chain.lock().await;
    // Never soft-adapt on gossip imports (that held the chain lock for seconds).
    match c.record_ai_receipt_imported(receipt) {
        Ok(true) => {
            let _ = c.credit_local_service(NodeServiceKind::AiRouting, 3);
            tracing::debug!(job_id, "imported peer ai result");
        }
        Ok(false) => {}
        Err(e) => warn!(error = %e, job_id, "reject ai result"),
    }
    Ok(())
}

async fn import_finality_attest(
    chain: &SharedChain,
    relay: &broadcast::Sender<RelayEvent>,
    att: FinalityAttestation,
) -> Result<()> {
    let fanout = att.clone();
    let mut c = chain.lock().await;
    match c.record_finality_attestation(att) {
        Ok(ing) => {
            if let Some(addr) = ing.slashed {
                let height = c.height();
                info!(address = %addr, "finality equivocation — bond slashed");
                drop(c);
                let _ = relay.send(RelayEvent::SlashMark {
                    address: addr.to_string(),
                    txid: String::new(),
                    height,
                    stake_atomic: 0,
                    peer_id: String::new(),
                });
                return Ok(());
            }
            if ing.new_vote {
                info!(
                    height = fanout.height,
                    advanced = ing.advanced,
                    "imported peer finality attest"
                );
                drop(c);
                let _ = relay.send(RelayEvent::FinalityAttest(fanout));
            }
        }
        Err(e) => tracing::debug!(error = %e, "reject finality attest"),
    }
    Ok(())
}

async fn import_slash_mark(
    chain: &SharedChain,
    address: &str,
    height: u64,
    stake_atomic: u64,
    peer_id: &str,
    txid: &str,
) -> Result<()> {
    let Some(addr) = Address::from_hex(address) else {
        warn!(address, "bad slashmark address");
        return Ok(());
    };
    let mut c = chain.lock().await;
    match c.apply_slash_mark(addr, height, stake_atomic, peer_id, txid) {
        Ok(rec) => {
            info!(
                address,
                height,
                stake_atomic,
                txid,
                preferred = !txid.trim().is_empty(),
                slashed = rec.slashed,
                "imported peer slashmark"
            );
        }
        Err(e) => warn!(error = %e, address, "reject slashmark"),
    }
    Ok(())
}

async fn import_block(
    chain: &SharedChain,
    relay: &broadcast::Sender<RelayEvent>,
    block: Block,
) -> Result<()> {
    let height = block.header.height;
    let id = block.id();
    let fanout = block.clone();
    let mut c = chain.lock().await;
    match c.import_block(block) {
        Ok(true) => {
            let _ = c.credit_local_service(NodeServiceKind::BlockRelay, 10);
            info!(height, %id, "imported peer block");
            drop(c);
            let _ = relay.send(RelayEvent::Block(fanout));
        }
        Ok(false) => {}
        Err(e) => warn!(error = %e, height, "reject block"),
    }
    Ok(())
}

async fn import_tx(
    chain: &SharedChain,
    _relay: &broadcast::Sender<RelayEvent>,
    tx: Transaction,
) -> Result<()> {
    let mut c = chain.lock().await;
    match c.submit_tx(tx) {
        Ok(id) => {
            let _ = c.credit_local_service(NodeServiceKind::TxRelay, 1);
            info!(%id, "imported peer tx");
        }
        Err(e) => tracing::debug!(error = %e, "ignore peer tx"),
    }
    Ok(())
}

pub fn load_or_create_identity(path: &Path) -> Result<identity::Keypair> {
    if path.exists() {
        let bytes = std::fs::read(path)?;
        let kp = identity::Keypair::from_protobuf_encoding(&bytes)
            .map_err(|e| anyhow::anyhow!("invalid p2p identity: {e}"))?;
        return Ok(kp);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let kp = identity::Keypair::generate_ed25519();
    let bytes = kp
        .to_protobuf_encoding()
        .map_err(|e| anyhow::anyhow!("encode identity: {e}"))?;
    std::fs::write(path, bytes)?;
    info!(path = %path.display(), "wrote libp2p identity");
    Ok(kp)
}

/// Parse `host:port`, `/ip4/.../udp/.../quic-v1`, or full multiaddr with `/p2p/`.
/// Hostnames are resolved via DNS (first A/AAAA).
/// Bare hostnames (no port) default to the official seed P2P port.
pub fn parse_listen_addr(s: &str) -> Result<Multiaddr> {
    if s.starts_with('/') {
        return Ok(Multiaddr::from_str(s)?);
    }
    let s = s.trim();
    let with_port = if s.parse::<std::net::SocketAddr>().is_ok() || s.contains(':') {
        s.to_string()
    } else {
        format!("{}:{}", s, mesh_types::SEED_P2P_PORT)
    };
    if let Ok(sock) = with_port.parse::<std::net::SocketAddr>() {
        return socket_to_quic_multiaddr(sock);
    }
    let mut addrs = with_port
        .to_socket_addrs()
        .with_context(|| format!("resolve seed/listen addr {with_port}"))?;
    let sock = addrs
        .next()
        .ok_or_else(|| anyhow::anyhow!("no DNS addresses for {with_port}"))?;
    socket_to_quic_multiaddr(sock)
}

pub fn parse_seed_addr(s: &str) -> Result<Multiaddr> {
    parse_listen_addr(s)
}

pub fn socket_to_quic_multiaddr(sock: std::net::SocketAddr) -> Result<Multiaddr> {
    let mut ma = Multiaddr::empty();
    match sock {
        std::net::SocketAddr::V4(v4) => {
            ma.push(Protocol::Ip4(*v4.ip()));
            ma.push(Protocol::Udp(v4.port()));
            ma.push(Protocol::QuicV1);
        }
        std::net::SocketAddr::V6(v6) => {
            ma.push(Protocol::Ip6(*v6.ip()));
            ma.push(Protocol::Udp(v6.port()));
            ma.push(Protocol::QuicV1);
        }
    }
    Ok(ma)
}
