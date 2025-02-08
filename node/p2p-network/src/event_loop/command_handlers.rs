use libp2p::{
    kad,
    PeerId
};
use color_eyre::eyre::anyhow;
use colored::Colorize;
use crate::node_client::NodeCommand;
use crate::types::{
    FragmentRequestEnum,
    FragmentResponseEnum,
    GossipTopic,
    TopicHash,
    UmbralPeerId
};
use crate::short_peer_id;
use crate::types::{CapsuleFragmentMessage, KeyFragmentMessage};
use crate::SendError;
use super::EventLoop;


impl<'a> EventLoop<'a> {

    pub(crate) async fn handle_command(&mut self, command: NodeCommand) {
        match command {
            NodeCommand::SubscribeTopics { topics, sender } => {
                let subscribed_topics = self.subscribe_topics(&topics);
                sender.send(subscribed_topics).ok();
            }
            NodeCommand::UnsubscribeTopics { topics, sender } => {
                let unsubscribed_topics = self.unsubscribe_topics(&topics);
                sender.send(unsubscribed_topics).ok();
            }
            NodeCommand::BroadcastKfrags(key_fragment_message) => {

                let message = key_fragment_message;

                let agent_name_nonce = match message.topic.clone() {
                    GossipTopic::BroadcastKfrag(agent_name_nonce, _, _) => {
                        agent_name_nonce
                    }
                    _ => {
                        panic!("Message type must be GossipTopic::BroadcastKfrag, received: {}", message.topic);
                    }
                };
                // set broadcaster's peer info
                self.peer_manager.set_peer_info_agent_vessel(
                    &agent_name_nonce,
                    // &message.threshold,
                    &message.total_frags,
                    message.vessel_peer_id, // vessel_peer_id
                    message.next_vessel_peer_id // next_vessel_peer_id
                );

                self.log(format!("Broadcasting KeyFrag topic: {}", message.topic));
                let match_topic = message.topic.to_string();

                let kfrag_indexed: KeyFragmentMessage = message.into();
                let kfrag_indexed_bytes = serde_json::to_vec(&kfrag_indexed)
                    .expect("serde err");

                match self.topics.get(&match_topic) {
                    Some(topic) => {
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .publish(topic.clone(), kfrag_indexed_bytes)
                            .map_err(|e| anyhow!(e.to_string()))
                            .ok();
                    }
                    None => {
                        self.log(format!("Topic '{}' not found in subscribed topics.", match_topic));
                        self.print_subscribed_topics();
                    }
                }
            }
            NodeCommand::GetKfragBroadcastPeers { agent_name_nonce, sender } => {
                match self.peer_manager.get_all_kfrag_peers(&agent_name_nonce) {
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
            NodeCommand::RequestFragment {
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
                        self.peer_id
                    ));

                self.pending.request_fragments.insert(request_id, sender);
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

                self.log(format!(
                    "\nAdding peer to kfrags_peers({}, {}, {})",
                    agent_name_nonce,
                    frag_num,
                    short_peer_id(&sender_peer_id)
                ).bright_green());

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
            NodeCommand::GetPeerUmbralPublicKeys { sender, agent_name_nonce } => {

                let public_keys = self.peer_manager
                    .get_connected_peers()
                    .iter()
                    .map(|(peer_id, _peer_info)| peer_id)
                    .collect::<Vec<&PeerId>>();

                for peer_id in public_keys {
                    let umbral_pk_kademlia_key = UmbralPeerId::from(peer_id);
                    let _query_id = self.swarm.behaviour_mut()
                        .kademlia
                        .get_record(kad::RecordKey::new(&umbral_pk_kademlia_key.to_string()));

                    self.pending.get_umbral_pks.insert(umbral_pk_kademlia_key, sender.clone());
                };
            }
            NodeCommand::RespondCfrag {
                agent_name_nonce,
                frag_num,
                sender_peer_id,
                channel
            } => {

                self.log(format!("RespondCfrags for: {agent_name_nonce}"));
                match self.peer_manager.get_cfrags(&agent_name_nonce) {
                    None => {},
                    // Do not send if no cfrag found, fastest successful futures returns
                    // with futures::future:select_ok()
                    Some(cfrag) => {
                        self.log(format!("RespondCfrags: found cfrag: {:?}", cfrag.verifying_pk));

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
            NodeCommand::SwitchTopic(ts, sender) => {

                self.broadcast_topic_switch(&ts).await;

                let agent_name_nonce = ts.next_topic.agent_name_nonce;
                let total_frags = ts.next_topic.total_frags;

                // get all peers subscribe to topic
                let all_peers = self.swarm
                    .behaviour_mut().gossipsub.all_peers()
                    .collect::<Vec<(&PeerId, Vec<&TopicHash>)>>();

                let peers = all_peers
                    .into_iter()
                    .filter_map(|(peer_id, peers_topics)| {

                        let topic1: TopicHash = GossipTopic::BroadcastKfrag(
                            agent_name_nonce.clone(),
                            total_frags,
                            0
                        ).into();

                        if peers_topics.contains(&&topic1.into()) {
                            Some(peer_id)
                        } else {
                            None
                        }
                    }).collect::<Vec<&PeerId>>();

                // send peer len and fragment info so we know if we can broadcast fragments
                sender.send(peers.len()).ok();
            }
            NodeCommand::StartListening { addr, sender } => {
                let _ = match self.swarm.listen_on(addr) {
                    Ok(_) => sender.send(Ok(())),
                    Err(e) => sender.send(Err(Box::new(e))),
                };
            }
            NodeCommand::TriggerRestart { sender, reason } => {
                self.log(format!("Restart Triggered. Reason: {:?}", reason).magenta());
                sender.send(reason.clone()).ok();
                std::thread::sleep(std::time::Duration::from_millis(1000));
                // self.handle_internal_heartbeat_failure(heartbeat)
                self.container_manager.trigger_restart(reason).await.ok();
            }
        }

    }
}