use color_eyre::Result;
use futures::FutureExt;
use libp2p::PeerId;
use tracing::{info, debug, error, warn};

use crate::short_peer_id;
use crate::network_events::{
    NodeIdentity,
    peer_manager::peer_info::AgentVesselInfo
};
use crate::types::{
    NetworkEvent,
    Reverie,
    ReverieType,
    ReverieId,
};
use crate::SendError;
use super::{NodeClient, NodeCommand};


impl<'a> NodeClient<'a> {
    pub async fn listen_to_network_events(
        &mut self,
        mut network_event_receiver: tokio::sync::mpsc::Receiver<NetworkEvent>
    ) -> Result<()> {
        loop {
            tokio::select! {
                e = network_event_receiver.recv() => match e {
                    // Reply with the content of the file on incoming requests.
                    Some(NetworkEvent::InboundCapsuleFragRequest {
                        reverie_id,
                        frag_num,
                        kfrag_provider_peer_id,
                        channel
                    }) => {
                        self.command_sender
                            .send(NodeCommand::RespondCapsuleFragment {
                                reverie_id,
                                frag_num,
                                kfrag_provider_peer_id,
                                channel
                            })
                            .await
                            .expect("Command receiver not to be dropped.");
                    }
                    Some(NetworkEvent::RespawnRequest(AgentVesselInfo {
                        agent_name_nonce,
                        total_frags,
                        next_vessel_peer_id,
                        current_vessel_peer_id
                    })) => {
                        self.handle_respawn_request(
                            agent_name_nonce,
                            total_frags,
                            next_vessel_peer_id,
                            current_vessel_peer_id
                        ).await?;
                    }
                    e => panic!("Error <network_event_receiver>: {:?}", e),
                }
                // hb = self.heartbeat_receiver.recv() => match hb {
                //     Ok(r) => info!("hb: {:?}", r),
                //     Err(e) => error!("err: {:?}", e),
                // },
            }
        }
    }

}

