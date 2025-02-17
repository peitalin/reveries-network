use color_eyre::owo_colors::OwoColorize;
use libp2p::{
    mdns,
    swarm::SwarmEvent
};
use crate::behaviour::BehaviourEvent;
use super::NetworkEvents;


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
                    self.log(tee_log_str);
                }
            }

            //// mDNS Peer Discovery events
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns_event)) => {
                match mdns_event {
                    mdns::Event::Discovered(list) => {
                        for (peer_id, multiaddr) in list {
                            self.log(format!("mDNS adding peer {:?}", peer_id));
                            self.swarm.behaviour_mut().kademlia.add_address(&peer_id, multiaddr);
                        }
                    }
                    mdns::Event::Expired(list) => {
                        for (peer_id, multiaddr) in list {
                            self.swarm.behaviour_mut().kademlia.remove_address(&peer_id, &multiaddr);
                        }
                    }
                }
            }

            //// Swarm connection events
            SwarmEvent::NewListenAddr { .. } => {}
            SwarmEvent::Dialing { .. } => {}
            SwarmEvent::IncomingConnection { .. } => {},
            SwarmEvent::ConnectionEstablished { .. } => { }
            SwarmEvent::ExpiredListenAddr { .. } => { }
            SwarmEvent::IncomingConnectionError { .. } => { }
            SwarmEvent::OutgoingConnectionError { .. } => { }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                self.log(format!("ConnectionClosed with peer: {:?}", peer_id));
                // TODO: Do not remove vessel peers if connection is lost right away.
                // put them in a queue for reincarnating and remove only after they
                // have been reincarnated in a new vessel

                // self.remove_peer(&peer_id);
            }

            //// Unhandled SwarmEvents
            swarm_event => println!("Unhandled SwarmEvent: {swarm_event:?}"),
        }
    }

}

