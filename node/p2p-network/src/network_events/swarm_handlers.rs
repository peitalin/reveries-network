use libp2p::{
    kad,
    swarm::SwarmEvent,
    Multiaddr,
    multiaddr,
};
use colored::Colorize;
use tracing::{trace, info, warn, debug};
use crate::behaviour::BehaviourEvent;
use super::NetworkEvents;
use serde_json;


impl<'a> NetworkEvents<'a> {
    pub(super) async fn handle_swarm_event(&mut self, swarm_event: SwarmEvent<BehaviourEvent>) {
        match swarm_event {

            //// Kademlia events for storing Umbral PRE pubkeys
            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(kademlia_event)) => {
                self.handle_kademlia_event(kademlia_event).await;
            }

            //// Request Response events for transferring frags
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(rr_event)) => {
                self.handle_request_response(rr_event).await
            },

            //// GossipSub events for PRE broadcasts
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossip_event)) => {
                self.handle_gossipsub_event(gossip_event).await;
            }

            //// Heartbeat Protocol events
            SwarmEvent::Behaviour(BehaviourEvent::Heartbeat(tee_event)) => {
                self.peer_manager.update_peer_heartbeat(tee_event.peer_id, tee_event.latest_tee_attestation);
                if let Some(tee_log_str) = self.peer_manager.make_heartbeat_tee_log(tee_event.peer_id) {
                    info!("{} {}", self.nname(), tee_log_str);
                }
            }

            //// Identify events
            SwarmEvent::Behaviour(BehaviourEvent::Identify(libp2p_identify::Event::Received { peer_id, info, .. })) => {
                info!("{} Identified peer: {:?}", self.nname(), peer_id);
                info!("{} Peer protocols: {:?}", self.nname(), info.protocols);

                // Add their listen addresses to Kademlia
                for addr in info.listen_addrs {
                    self.swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                }

                // Extract peer ID from observed address if it contains one
                if let Some(multiaddr::Protocol::P2p(observed_peer_id)) = info.observed_addr
                    .iter()
                    .find(|p| matches!(p, multiaddr::Protocol::P2p(_)))
                {
                    if self.should_dial_peer(&observed_peer_id) {
                        if let Err(e) = self.swarm.dial(observed_peer_id) {
                            warn!("{} Failed to dial identified peer: {}", self.nname(), e);
                        }
                    }
                }
            }

            //// Swarm connection events
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("{} {}", self.nname(), format!("New listen address: {}", address));
            }
            SwarmEvent::Dialing { peer_id, .. } => {
                info!("{} {}", self.nname(), format!("Dialing peer: {:?}", peer_id));
            }
            SwarmEvent::IncomingConnection { local_addr, send_back_addr, .. } => {
                info!("{} {}", self.nname(), format!("Incoming connection from {} to {}", send_back_addr, local_addr));
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                info!("{} {}", self.nname(), format!("Connection established with peer: {:?}", peer_id));

                // Add peer to peer manager
                self.peer_manager.insert_peer_info(peer_id);

                // Bootstrap to find more peers
                self.swarm.behaviour_mut().kademlia.bootstrap().ok();

                // Query for our own peer ID to find peers close to us
                self.swarm.behaviour_mut().kademlia.get_closest_peers(self.peer_id);

                // Also query for the new peer's closest peers
                self.swarm.behaviour_mut().kademlia.get_closest_peers(peer_id);

                // Start providing our peer ID
                self.swarm.behaviour_mut().kademlia.start_providing(kad::RecordKey::new(&self.peer_id.to_string())).ok();
            }

            SwarmEvent::ExpiredListenAddr { .. } => { }
            SwarmEvent::IncomingConnectionError { .. } => { }

            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                warn!("{} {}", self.nname(), format!("OutgoingConnectionError with peer: {:?}, error: {:?}", peer_id, error).red());
                // Log the error but don't remove addresses - let Kademlia handle its own routing table
            }

            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                info!("{} {}", self.nname(), format!("ConnectionClosed peer: {:?}", peer_id).red());

                // Remove from peer manager when connection closes
                self.peer_manager.remove_peer_info(&peer_id);
            }

            //// Unhandled SwarmEvents
            swarm_event => trace!("Unhandled SwarmEvent: {swarm_event:?}"),
        }
    }
}

