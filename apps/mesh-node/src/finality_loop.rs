//! Local auto-attest when this node's wallet holds a finality-eligible bond.

use std::time::Duration;

use mesh_crypto::Keypair;
use mesh_p2p::{NetworkHandle, SharedChain};
use tracing::info;

pub async fn run(chain: SharedChain, net: NetworkHandle, kp: Keypair) {
    info!(wallet = %kp.address(), "finality auto-attest armed (no-op while gate is off)");
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let atts = {
            let mut c = chain.lock().await;
            match c.maybe_local_attest(&kp) {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!(error = %e, "local finality attest skipped");
                    Vec::new()
                }
            }
        };
        for att in atts {
            info!(height = att.height, "local finality attest");
            net.announce_finality_attest(att);
        }
    }
}
