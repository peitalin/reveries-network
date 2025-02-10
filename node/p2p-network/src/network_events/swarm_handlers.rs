use libp2p::{
    mdns,
    swarm::SwarmEvent
};
use crate::behaviour::BehaviourEvent;
use super::NetworkEvents;


impl<'a> NetworkEvents<'a> {
    pub(super) async fn handle_swarm_event(&mut self, swarm_event: SwarmEvent<BehaviourEvent>) {
        match swarm_event {

            //// Kademlia
            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(kad_event)) => {
                self.handle_kademlia_event(kad_event);
            }

            //// Request Response Protocol
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                request_response_event
            )) => {
                self.handle_request_response(request_response_event).await
            },


            //////////////////////////////////
            //// GossipSub protocol for PRE
            //////////////////////////////////
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gevent)) => {
                self.handle_gossipsub_event(gevent).await;
            }

            SwarmEvent::Behaviour(BehaviourEvent::Heartbeat(tee_event)) => {
                self.peer_manager.update_peer_heartbeat(tee_event.peer_id, tee_event.latest_tee_attestation);
                if let Some(tee_str) = self.peer_manager.log_heartbeat_tee(tee_event.peer_id) {
                    self.log(tee_str);
                }
            }

            //// Connections
            SwarmEvent::NewListenAddr { .. } => {}
            SwarmEvent::Dialing { .. } => {}
            SwarmEvent::IncomingConnection { .. } => {},
            SwarmEvent::ConnectionEstablished { .. } => { }
            SwarmEvent::ExpiredListenAddr { .. } => { }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                println!(">>> ConnectionClosed with peer: {:?}", peer_id);
                self.remove_peer(&peer_id);
            }
            SwarmEvent::IncomingConnectionError { .. } => { }
            SwarmEvent::OutgoingConnectionError { .. } => { }
            //// mDNS Protocol
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                for (peer_id, multiaddr) in list {
                    if !self.container_manager.read().await.simulate_network_failure {
                        self.log(format!("mDNS adding peer {:?}", peer_id));
                        self.swarm.behaviour_mut().kademlia.add_address(&peer_id, multiaddr);
                    } else {
                        self.log("Simulating network failure: blocking mDNS peer adding attempt");
                    }
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                for (peer_id, multiaddr) in list {
                    self.swarm.behaviour_mut().kademlia.remove_address(&peer_id, &multiaddr);
                }
            },

            swarm_event => println!("Unhandled SwarmEvent: {swarm_event:?}"),
        }
    }

}

