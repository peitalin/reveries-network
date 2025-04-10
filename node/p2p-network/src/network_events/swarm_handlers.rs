use color_eyre::Result;
use colored::Colorize;
use libp2p::swarm::SwarmEvent;
use libp2p::swarm::DialError;
use tracing::{trace, info, warn, debug};

use crate::behaviour::BehaviourEvent;
use crate::get_node_name2;
use super::NetworkEvents;


impl<'a> NetworkEvents<'a> {
    pub(super) async fn handle_swarm_event(&mut self, swarm_event: SwarmEvent<BehaviourEvent>) -> Result<()> {
        match swarm_event {

            //// Kademlia events for storing Umbral PRE pubkeys
            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(kademlia_event)) => {
                self.handle_kademlia_event(kademlia_event).await?;
            }

            //// Request Response events for transferring frags
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(rr_event)) => {
                self.handle_request_response(rr_event).await?;
            },

            //// GossipSub events for PRE broadcasts
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossip_event)) => {
                self.handle_gossipsub_event(gossip_event).await?;
            }

            //// Heartbeat Protocol events
            SwarmEvent::Behaviour(BehaviourEvent::Heartbeat(tee_event)) => {
                self.peer_manager.update_peer_heartbeat(
                    tee_event.peer_id,
                    tee_event.latest_tee_attestation
                );

                if let Some(tee_str) = self.peer_manager.make_heartbeat_tee_log(tee_event.peer_id) {
                    info!("{} {}", self.nname(), tee_str);
                }
                // prove node has TEE attestation before accepting peer
                // market peer as "TEE: verified" when getting providers
            }

            //// Identify events for peer discovery via bootstrap node
            SwarmEvent::Behaviour(BehaviourEvent::Identify(
                libp2p_identify::Event::Received { peer_id, info, .. }
            )) => {
                info!("{} Identified and adding peer: {:?}", self.nname(), peer_id);
                for addr in info.listen_addrs {
                    self.swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                }
            }

            //// Swarm connection events
            SwarmEvent::NewListenAddr { address, .. } => {
                debug!("{} {}", self.nname(), format!("New listen address: {}", address));
            }
            SwarmEvent::Dialing { peer_id, .. } => {
                debug!("{} {}", self.nname(), format!("Dialing peer: {:?}", peer_id));
            }
            SwarmEvent::IncomingConnection { local_addr, send_back_addr, .. } => {
                debug!("{} {}", self.nname(), format!("Incoming connection from {} to {}", send_back_addr, local_addr));
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                debug!("{} {}", self.nname(), format!("Connection established with peer: {:?}", peer_id));
                self.peer_manager.insert_peer_info(peer_id);
            }
            SwarmEvent::ExpiredListenAddr { .. } => { }
            SwarmEvent::IncomingConnectionError { .. } => { }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                let err_msg = match error {
                    DialError::Transport(transport_errors) => {
                        let error_msg = transport_errors.iter().next().unwrap().1.to_string();
                        format!("{}", error_msg).red()
                    }
                    _ => {
                        format!("{}", error).red()
                    }
                };
                tracing::error!("{} OutgoingConnectionError with peer: {:?} {}", self.nname(), peer_id, err_msg);
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                info!("{} {}", self.nname(), format!("ConnectionClosed peer: {:?}", peer_id));
                // Remove from peer manager after heartbeat timeout, not when connection closes
            }
            //// Unhandled SwarmEvents
            swarm_event => trace!("Unhandled SwarmEvent: {swarm_event:?}"),
        }

        Ok(())
    }
}

