use libp2p::{
    mdns,
    kad,
    Multiaddr,
    swarm::SwarmEvent
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

            // //// mDNS Peer Discovery events
            // SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns_event)) => {
            //     match mdns_event {
            //         mdns::Event::Discovered(list) => {
            //             for (peer_id, multiaddr) in list {
            //                 info!("{} {}", self.nname(), format!("mDNS discovered peer {:?} at {:?}", peer_id, multiaddr));
            //                 self.swarm.behaviour_mut().kademlia.add_address(&peer_id, multiaddr.clone());

            //                 // Add to peer manager when discovered
            //                 self.peer_manager.insert_peer_info(peer_id);

            //                 // Try to connect if we're not already connected
            //                 if self.should_dial_peer(&peer_id) {
            //                     if let Err(e) = self.swarm.dial(peer_id) {
            //                         warn!("{} Failed to dial discovered peer: {}", self.nname(), e);
            //                     }
            //                 }
            //             }
            //         }
            //         mdns::Event::Expired(list) => {
            //             for (peer_id, multiaddr) in list {
            //                 info!("{} {}", self.nname(), format!("mDNS expired peer {:?} at {:?}", peer_id, multiaddr));
            //                 self.swarm.behaviour_mut().kademlia.remove_address(&peer_id, &multiaddr);
            //             }
            //         }
            //     }
            // }

            //// Swarm connection events
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("{} {}", self.nname(), format!("New listen address: {}", address));

                // Store our new address in the DHT
                let record_key = kad::RecordKey::new(&self.peer_id.to_string());
                let our_addresses: Vec<_> = self.swarm.listeners().cloned().collect();

                if let Err(e) = self.swarm.behaviour_mut().kademlia.put_record(
                    kad::Record {
                        key: record_key,
                        value: serde_json::to_vec(&our_addresses).unwrap(),
                        publisher: Some(self.peer_id),
                        expires: None,
                    },
                    kad::Quorum::One
                ) {
                    warn!("{} Failed to put record: {}", self.nname(), e);
                }
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

                // Get our listen addresses and publish them
                let our_addresses: Vec<Multiaddr> = self.swarm.listeners().cloned().collect();
                if !our_addresses.is_empty() {
                    let key = kad::RecordKey::new(&format!("addr:{}", self.peer_id));
                    let value = serde_json::to_vec(&our_addresses).unwrap_or_default();

                    self.swarm.behaviour_mut().kademlia.put_record(
                        kad::Record {
                            key,
                            value,
                            publisher: Some(self.peer_id),
                            expires: None,
                        },
                        kad::Quorum::One
                    ).ok();
                }

                // Also try to get the peer's stored records
                let peer_addr_key = kad::RecordKey::new(&format!("addr:{}", peer_id));
                self.swarm.behaviour_mut().kademlia.get_record(peer_addr_key);
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

