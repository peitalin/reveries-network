use color_eyre::Result;
use futures::FutureExt;
use libp2p::{PeerId, Multiaddr};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, error};
use p256::ecdsa::VerifyingKey as P256VerifyingKey;

use crate::env_var::EnvVars;
use crate::{short_peer_id, SendError};
use crate::network_events::NodeIdentity;
use crate::types::{
    NetworkEvent,
    Reverie,
    ReverieType,
    ReverieId,
    AgentVesselInfo
};
use super::{NodeClient, NodeCommand};


impl NodeClient {

    pub async fn start_listening_to_network(&mut self, listen_addresses: Vec<Multiaddr>) -> Result<()> {

        let listen_addresses = if listen_addresses.is_empty() {
            vec!["/ip4/0.0.0.0/tcp/0".parse()?]
        } else {
            listen_addresses
        };
        info!("Starting to listen to network on: {:?}", listen_addresses);

        for addr in listen_addresses {
            let (sender, receiver) = oneshot::channel();
            // In case a listen address was provided use it, otherwise listen on any address.
            self.command_sender
                .send(NodeCommand::StartListening {
                    addr,
                    sender
                })
                .await?;

            let res = receiver.await?.map_err(SendError::from)?;
            info!("Listening to network on: {:?}", res);
        };
        Ok(())
    }

    pub async fn listen_to_network_events(
        &mut self,
        mut network_event_receiver: mpsc::Receiver<NetworkEvent>
    ) -> Result<()> {
        loop {
            tokio::select! {
                event = network_event_receiver.recv() => match event {
                    Some(NetworkEvent::RespawnRequest(
                        AgentVesselInfo {
                            reverie_id,
                            reverie_type,
                            threshold,
                            total_frags,
                            next_vessel_peer_id, // This node is the next vessel
                            current_vessel_peer_id: prev_failed_vessel_peer_id // Previous (failed) vessel
                        }
                    )) => {
                        match self.handle_respawn_request(
                            reverie_id,
                            reverie_type,
                            threshold,
                            total_frags,
                            next_vessel_peer_id,
                            prev_failed_vessel_peer_id
                        ).await {
                            Ok(_) => info!("Respawn request handled successfully"),
                            Err(e) => error!("Error handling respawn request: {:?}", e),
                        };
                    }
                    event => panic!("Error <network_event_receiver>: {:?}", event),
                }
            }
        }
    }

}

