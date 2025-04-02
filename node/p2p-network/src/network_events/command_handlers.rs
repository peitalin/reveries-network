use libp2p::{
    kad,
    PeerId
};
use color_eyre::eyre::anyhow;
use tracing::info;

use crate::node_client::NodeCommand;
use crate::types::{
    AgentVesselInfo,
    FragmentRequestEnum,
    FragmentResponseEnum,
    GossipTopic,
    TopicHash,
    NodeVesselStatus,
    VesselStatus,
    VesselPeerId
};
use crate::short_peer_id;
use crate::types::{CapsuleFragmentMessage, KeyFragmentMessage};
use crate::SendError;
use super::NetworkEvents;


impl<'a> NetworkEvents<'a> {
    pub(crate) async fn handle_command(&mut self, command: NodeCommand) {
        match command {
            NodeCommand::GetPeerNodeVesselStatuses { sender, .. } => {

                let public_keys = self.swarm.connected_peers()
                    .cloned()
                    .collect::<Vec<PeerId>>();

                for peer_id in public_keys {
                    // add prefix as kademlia key
                    let vessel_kademlia_key = VesselPeerId::from(peer_id);
                    self.swarm.behaviour_mut()
                        .kademlia
                        .get_record(kad::RecordKey::new(&vessel_kademlia_key.to_string()));

                    self.pending.get_node_vessel_status.insert(vessel_kademlia_key, sender.clone());
                };
            }
            NodeCommand::GetKfragBroadcastPeers { agent_name_nonce, sender } => {
                match self.peer_manager.get_kfrag_broadcast_peers(&agent_name_nonce) {
                    None => {
                        println!("missing kfrag_peers: {:?}", self.peer_manager.kfrag_broadcast_peers);
                        sender.send(std::collections::HashMap::new()).ok();
                    }
                    Some(peers) => {
                        // unsorted
                        sender.send(peers.clone()).ok();
                    }
                }
            }
            NodeCommand::GetKfragProviders { agent_name_nonce, sender } => {
                let query_id = self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .get_providers(agent_name_nonce.to_string().into_bytes().into());

                self.pending.get_providers.insert(query_id, sender);
            }
            NodeCommand::SaveKfragProvider {
                agent_name_nonce,
                frag_num,
                sender_peer_id,
                channel
            } => {
                info!(
                    "\n{} Adding peer to kfrags_peers({}, {}, {})",
                    self.nname(),
                    agent_name_nonce,
                    frag_num,
                    short_peer_id(&sender_peer_id)
                );

                self.save_peer(&sender_peer_id, agent_name_nonce, frag_num);

                // confirm saved kfrag peer
                self.swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(
                        channel,
                        FragmentResponseEnum::KfragProviderAck
                    )
                    .expect("Connection to peer to be still open.");
            }
            NodeCommand::BroadcastSwitchTopic(ts, sender) => {

                self.broadcast_topic_switch(&ts).await.ok();

                // get all peers subscribe to topic
                let all_peers = self.swarm
                    .behaviour_mut().gossipsub.all_peers()
                    .collect::<Vec<(&PeerId, Vec<&TopicHash>)>>();

                let topic_compare: TopicHash = GossipTopic::BroadcastKfrag(
                    ts.next_topic.agent_name_nonce,
                    ts.next_topic.total_frags,
                    0
                ).into();

                let peers = all_peers
                    .into_iter()
                    .filter_map(|(peer_id, peers_topics)| {
                        match peers_topics.contains(&&topic_compare) {
                            true => Some(peer_id),
                            false => None
                        }
                    }).collect::<Vec<&PeerId>>();

                // send peer len and fragment info so we know if we can broadcast fragments
                sender.send(peers.len()).ok();
            }
            NodeCommand::BroadcastKfrags(msg) => {

                let topic_kfrag = msg.topic.to_string();
                let agent_name_nonce = match msg.topic.clone() {
                    GossipTopic::BroadcastKfrag(agent_name_nonce, _, _) => {
                        agent_name_nonce
                    }
                    _ => {
                        panic!("Message must be GossipTopic::BroadcastKfrag, got: {}", msg.topic);
                    }
                };
                let agent_vessel_info = AgentVesselInfo {
                    agent_name_nonce: agent_name_nonce.clone(),
                    total_frags: msg.total_frags,
                    current_vessel_peer_id: msg.vessel_peer_id,
                    next_vessel_peer_id: msg.next_vessel_peer_id,
                };

                // set broadcaster's peer info
                self.peer_manager.set_peer_info_agent_vessel(&agent_vessel_info);

                // Update vessel status on Kademlia DHT
                self.swarm
                    .behaviour_mut()
                    .kademlia
                    .put_record(
                        kad::Record {
                            key: kad::RecordKey::new(&VesselPeerId::from(msg.vessel_peer_id).to_string()),
                            value: serde_json::to_vec(
                                    &NodeVesselStatus {
                                        peer_id: msg.vessel_peer_id,
                                        umbral_public_key: msg.alice_pk, // vessel's public key
                                        agent_vessel_info: Some(agent_vessel_info),
                                        vessel_status: VesselStatus::ActiveVessel,
                                    }
                                ).expect("serde_json::to_vec(node_vessel_status)"),
                            publisher: Some(msg.vessel_peer_id),
                            expires: None,
                        },
                        kad::Quorum::One
                ).expect("put_record err");

                let kfrag_msg = serde_json::to_vec(&KeyFragmentMessage::from(msg))
                    .expect("serde err");

                match self.topics.get(&topic_kfrag) {
                    Some(topic) => {
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .publish(topic.clone(), kfrag_msg)
                            .map_err(|e| anyhow!(e.to_string()))
                            .ok();
                    }
                    None => {
                        info!("{} Topic '{}' not found in subscribed topics.", self.nname(), topic_kfrag);
                        self.print_subscribed_topics();
                    }
                }
            }
            NodeCommand::RequestCapsuleFragment {
                agent_name_nonce,
                frag_num,
                peer,
                sender,
            } => {
                let request_id = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer, FragmentRequestEnum::FragmentRequest(
                        agent_name_nonce,
                        frag_num,
                        self.node_id.peer_id
                    ));

                self.pending.request_fragments.insert(request_id, sender);
            }
            NodeCommand::RespondCapsuleFragment {
                agent_name_nonce,
                frag_num,
                sender_peer_id,
                channel
            } => {
                info!("{} RespondCapsuleFragment for {agent_name_nonce} frag_num: {frag_num}", self.nname());
                match self.peer_manager.get_cfrags(&agent_name_nonce) {
                    None => {},
                    // Do not send if no cfrag found, fastest successful futures returns
                    // with futures::future:select_ok()
                    Some(cfrag) => {
                        let cfrag_indexed_bytes =
                            serde_json::to_vec::<Option<CapsuleFragmentMessage>>(&Some(cfrag.clone()))
                                .map_err(|e| SendError(e.to_string()));

                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_response(
                                channel,
                                FragmentResponseEnum::FragmentResponse(cfrag_indexed_bytes)
                            )
                            .expect("Connection to peer to be still open.");
                    }
                }
            }
            NodeCommand::StartListening { addr, sender } => {
                let _ = match self.swarm.listen_on(addr) {
                    Ok(_) => sender.send(Ok(())),
                    Err(e) => sender.send(Err(Box::new(e))),
                };
            }
            NodeCommand::SubscribeTopics { topics, sender } => {
                let subscribed_topics = self.subscribe_topics(&topics);
                sender.send(subscribed_topics).ok();
            }
            NodeCommand::UnsubscribeTopics { topics, sender } => {
                let unsubscribed_topics = self.unsubscribe_topics(&topics);
                sender.send(unsubscribed_topics).ok();
            }
            NodeCommand::SimulateNodeFailure { sender, reason } => {
                info!("{} Simulating network failure:", self.nname());
                info!("Triggering heartbeat failure in 500ms: {:?}", reason);
                sender.send(reason.clone()).ok();
                std::thread::sleep(std::time::Duration::from_millis(500));
                self.simulate_heartbeat_failure().await;
            }
            NodeCommand::GetNodeState { sender } => {
                let node_state = self.query_node_state().await;
                sender.send(node_state).ok();
            }
            NodeCommand::GetListeningAddresses { sender } => {
                let addresses = self.swarm.listeners().cloned().collect();
                sender.send(addresses).ok();
            }
            NodeCommand::GetConnectedPeers { sender } => {
                let peers = self.swarm.connected_peers().cloned().collect();
                sender.send(peers).ok();
            }
        }
    }
}
