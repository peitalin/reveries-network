use libp2p::{
    kad,
    PeerId
};
use color_eyre::eyre::anyhow;
use tracing::{info, error};

use crate::node_client::NodeCommand;
use crate::types::{
    AgentVesselInfo,
    FragmentRequestEnum,
    FragmentResponseEnum,
    GossipTopic,
    TopicHash,
    NodeVesselWithStatus,
    VesselStatus,
    VesselPeerId,
    AgentReverieId,
    CapsuleFragmentMessage,
    KeyFragmentMessage,
    ReverieKeyfragMessage,
    ReverieType,
    ReverieKeyfrag,
    ReverieCapsulefrag,
    Reverie,
};
use crate::short_peer_id;
use crate::SendError;
use super::NetworkEvents;


impl<'a> NetworkEvents<'a> {
    pub(crate) async fn handle_command(&mut self, command: NodeCommand) {
        match command {

            NodeCommand::SendKfrag(
                fragment_provider_peer_id, // Fragment Provider
                kfrag_msg,
                agent_name_nonce,
            ) => {

                // let metadata = match kfrag_msg.reverie_keyfrag.reverie_type {
                //     ReverieType::Agent => {
                //         // PUT kademlia record:
                //         // AgentName => ReverieId
                //         agent_name_nonce
                //     }
                //     _ => {
                //         None
                //     }
                // }

                let _request_id = self.swarm.behaviour_mut()
                    .request_response
                    .send_request(
                        &fragment_provider_peer_id,
                        FragmentRequestEnum::SaveFragmentRequest(
                            kfrag_msg,
                            agent_name_nonce
                        )
                    );
            }
            NodeCommand::SendReverie(
                peer_id, // Ciphertext Holder
                reverie_msg,
                agent_name_nonce,
            ) => {
                // Dispatch Reverie (ciphertext) to target vessel
                let _request_id = self.swarm.behaviour_mut()
                    .request_response
                    .send_request(
                        &peer_id,
                        FragmentRequestEnum::SaveCiphertextRequest(
                            reverie_msg,
                            None
                        )
                    );
            }
            NodeCommand::GetNodeVesselStatusesFromKademlia { sender, .. } => {

                let peers = self.swarm.connected_peers()
                    .cloned()
                    .collect::<Vec<PeerId>>();

                for peer_id in peers {
                    // add prefix as kademlia key
                    let vessel_kademlia_key = VesselPeerId::from(peer_id);
                    self.swarm.behaviour_mut()
                        .kademlia
                        .get_record(kad::RecordKey::new(&vessel_kademlia_key.to_string()));

                    self.pending.get_node_vessels.insert(vessel_kademlia_key, sender.clone());
                };
            }
            NodeCommand::GetAgentReverieId {
                agent_name_nonce,
                sender,
            } => {
                // add prefix as kademlia key
                let agent_reverie_key = AgentReverieId::from(agent_name_nonce.clone());

                self.swarm.behaviour_mut()
                    .kademlia
                    .get_record(kad::RecordKey::new(&agent_reverie_key.to_string()));

                self.pending.get_agent_reverie_id.insert(agent_reverie_key, sender);
            }
            NodeCommand::GetKfragProviders { reverie_id, sender } => {
                match self.peer_manager.kfrag_providers.get(&reverie_id) {
                    None => {
                        error!("missing kfrag_providers: {:?}", self.peer_manager.kfrag_providers);
                        sender.send(std::collections::HashMap::new()).ok();
                    }
                    Some(peers) => {
                        // unsorted
                        sender.send(peers.clone()).ok();
                    }
                }
            }
            NodeCommand::SaveKfragProvider {
                reverie_id,
                frag_num,
                kfrag_provider_peer_id,
                channel
            } => {
                info!(
                    "\n{} Adding peer to kfrags_providers({}, {}, {})",
                    self.nname(),
                    reverie_id,
                    frag_num,
                    short_peer_id(&kfrag_provider_peer_id)
                );

                // 1). Add to PeerManager locally on this node
                self.peer_manager.insert_kfrag_provider(kfrag_provider_peer_id.clone(), reverie_id, frag_num);
                self.peer_manager.insert_peer_info(kfrag_provider_peer_id.clone());

                // 2). Respond to broadcasting node and acknowledge receipt of Kfrag
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

                // add prefix as kademlia key
                let agent_reverie_key = AgentReverieId::from(agent_name_nonce.clone());
                // Put agent_name_nonce => reverie_id on DHT
                self.swarm.behaviour_mut().kademlia.put_record(
                    kad::Record {
                        key: kad::RecordKey::new(&agent_reverie_key.to_string()),
                        value: serde_json::to_vec(&msg.reverie_id).expect("serde_json::to_vec(reverie_id)"),
                        publisher: Some(self.node_id.peer_id),
                        expires: None,
                    },
                    kad::Quorum::Majority
                ).expect("put_record err");

                // Update vessel status on Kademlia DHT
                self.swarm.behaviour_mut().kademlia.put_record(
                    kad::Record {
                        key: kad::RecordKey::new(&VesselPeerId::from(msg.vessel_peer_id).to_string()),
                        value: serde_json::to_vec(
                                &NodeVesselWithStatus {
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
                            .ok();
                    }
                    None => {
                        info!("{} Topic '{}' not found in subscribed topics.", self.nname(), topic_kfrag);
                        self.print_subscribed_topics();
                    }
                }
            }
            NodeCommand::RequestCapsuleFragment {
                reverie_id,
                frag_num,
                peer_id,
                sender,
            } => {
                let request_id = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer_id, FragmentRequestEnum::GetFragmentRequest(
                        reverie_id,
                        frag_num,
                        self.node_id.peer_id
                    ));

                self.pending.request_fragments.insert(request_id, sender);
            }
            NodeCommand::RespondCapsuleFragment {
                reverie_id,
                frag_num,
                kfrag_provider_peer_id,
                channel
            } => {
                info!("{} RespondCapsuleFragment for {reverie_id} frag_num: {frag_num}", self.nname());
                match self.peer_manager.get_cfrags(&reverie_id) {
                    None => {},
                    // Do not send if no cfrag found, fastest successful futures returns
                    // with futures::future:select_ok()
                    Some(cfrag) => {
                        let cfrag_indexed_bytes =
                            serde_json::to_vec::<Option<ReverieCapsulefrag>>(&Some(cfrag.clone()))
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
