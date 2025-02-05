use libp2p::{
    kad,
    PeerId
};
use color_eyre::eyre::anyhow;
use crate::node_client::NodeCommand;
use crate::types::{
    FragmentRequest, FragmentResponse, GossipTopic, UmbralPeerId,
    TopicHash,
};
use crate::types::{CapsuleFragmentIndexed, KeyFragmentMessage};
use crate::SendError;
use super::EventLoop;


impl EventLoop {

    pub(crate) async fn handle_command(&mut self, command: NodeCommand) {
        match command {
            NodeCommand::SubscribeTopics { topics, sender } => {
                let subscribed_topics = self.subscribe_topics(topics);
                let _ = sender.send(subscribed_topics);
            }
            NodeCommand::BroadcastKfrags(key_fragment_message) => {

                let message = key_fragment_message;
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
            NodeCommand::GetKfragPeers { agent_name, agent_nonce, sender } => {
                match self.peer_manager.get_all_kfrag_peers(&agent_name, agent_nonce) {
                    None => {
                        println!("missing kfrag_peers: {:?}", self.peer_manager.kfrags_peers);
                    }
                    Some(peers) => {
                        let _ = sender.send(peers.clone());
                    }
                }
            }
            NodeCommand::RequestFragment {
                agent_name,
                agent_nonce,
                frag_num,
                peer,
                sender,
            } => {
                let request_id = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer, FragmentRequest(agent_name, agent_nonce, frag_num, self.peer_id));

                self.pending.request_fragments.insert(request_id, sender);
            }
            NodeCommand::GetProviders { agent_name, agent_nonce, sender } => {
                let query_id = self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .get_providers(agent_name.into_bytes().into());

                self.pending.get_providers.insert(query_id, sender);
            }
            NodeCommand::GetPeerUmbralPublicKey { sender, agent_name, agent_nonce } => {

                let public_keys = self.peer_manager
                    .get_connected_peers()
                    .iter()
                    .map(|(peer_id, _peer_info)| {
                        peer_id
                    }).collect::<Vec<&PeerId>>();

                for peer_id in public_keys {
                    let umbral_pk_kademlia_key = UmbralPeerId::from(peer_id);
                    let _query_id = self.swarm.behaviour_mut()
                        .kademlia
                        .get_record(kad::RecordKey::new(&umbral_pk_kademlia_key.to_string()));

                    self.pending.get_umbral_pks.insert(umbral_pk_kademlia_key, sender.clone());
                };
            }
            NodeCommand::RespondCfrag {
                agent_name,
                agent_nonce,
                frag_num,
                sender_peer,
                channel
            } => {

                self.log(format!("RespondCfrags: finding topic for: {agent_name}-{agent_nonce}"));
                match self.peer_manager.get_cfrags(&agent_name, &agent_nonce) {
                    None => {},
                    // Do not send if no cfrag found, fastest successful futures returns
                    // with futures::future:select_ok()
                    Some(cfrag) => {
                        self.log(format!("RespondCfrags: found cfrag: {:?}", cfrag.verifying_pk));

                        let cfrag_indexed_bytes =
                            serde_json::to_vec::<Option<CapsuleFragmentIndexed>>(&Some(cfrag.clone()))
                                .map_err(|e| SendError(e.to_string()));

                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_response(channel, FragmentResponse(cfrag_indexed_bytes))
                            .expect("Connection to peer to be still open.");
                    }
                }
            }
            NodeCommand::SwitchTopic(ts, sender) => {

                let agent_name = ts.next_topic.agent_name.clone();
                let agent_nonce = ts.next_topic.agent_nonce.clone();
                let total_frags = ts.next_topic.total_frags.clone();
                // let frag_num = NODE_SEED_NUM
                //     .with(|n| total_frags % *n.borrow());

                self.broadcast_topic_switch(ts).await;

                // get all peers subscribe to topic
                let all_peers = self.swarm
                    .behaviour_mut().gossipsub.all_peers()
                    .collect::<Vec<(&PeerId, Vec<&TopicHash>)>>();

                let peers = all_peers
                    .into_iter()
                    .filter_map(|(peer_id, peers_topics)| {

                        let topic1: TopicHash = GossipTopic::BroadcastKfrag(
                            agent_name.clone(),
                            agent_nonce,
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
                sender.send(peers.len());
            }
            NodeCommand::StartListening { addr, sender } => {
                let _ = match self.swarm.listen_on(addr) {
                    Ok(_) => sender.send(Ok(())),
                    Err(e) => sender.send(Err(Box::new(e))),
                };
            }
            // NodeCommand::RespondFragment { fragment, channel } => {
            //     self.swarm
            //         .behaviour_mut()
            //         .request_response
            //         .send_response(channel, FragmentResponse(Ok(fragment)))
            //         .expect("Connection to peer to be still open.");
            // }
        }

    }
}