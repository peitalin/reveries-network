use color_eyre::Result;
use futures::FutureExt;
use libp2p::{PeerId, Multiaddr};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, error};

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


impl<'a> NodeClient<'a> {

    pub async fn start_listening_to_network(&mut self, listen_address: Option<Multiaddr>) -> Result<()> {
        let (sender, receiver) = oneshot::channel();
        // In case a listen address was provided use it, otherwise listen on any address.
        self.command_sender
            .send(NodeCommand::StartListening {
                addr: listen_address.unwrap_or("/ip4/0.0.0.0/tcp/0".parse()?),
                sender
            })
            .await?;

        receiver.await?.map_err(|e| SendError(e.to_string()).into())
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
                            total_frags,
                            threshold,
                            next_vessel_peer_id, // This node is the next vessel
                            current_vessel_peer_id: prev_failed_vessel_peer_id // Previous (failed) vessel
                        }
                    )) => {
                        self.handle_respawn_request(
                            reverie_id,
                            reverie_type,
                            total_frags,
                            threshold,
                            next_vessel_peer_id,
                            prev_failed_vessel_peer_id
                        ).await?;
                    }
                    event => panic!("Error <network_event_receiver>: {:?}", event),
                }
                // hb = self.heartbeat_receiver.recv() => match hb {
                //     Ok(r) => info!("hb: {:?}", r),
                //     Err(e) => error!("err: {:?}", e),
                // },
            }
        }
    }

}

