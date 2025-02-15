
use std::collections::HashMap;
use color_eyre::eyre::anyhow;
use libp2p::gossipsub;
use crate::{get_node_name, short_peer_id};
use crate::types::{
    AgentNameWithNonce,
    CapsuleFragmentMessage,
    FragmentRequestEnum,
    GossipTopic,
    KeyFragmentMessage,
    TopicSwitch,
};
use crate::create_network::NODE_SEED_NUM;
use super::NetworkEvents;



impl<'a> NetworkEvents<'a> {
    // When receiving a Gossipsub event/message
    pub async fn handle_gossipsub_event(&mut self, gevent: gossipsub::Event) {
        match gevent {
            gossipsub::Event::Message {
                propagation_source: propagation_peer_id,
                message_id,
                message,
            } => {

                self.log(format!(
                    "\n\tReceived message: '{}'\n\t|from peer: {} {}\n\t|topic: '{}'\n",
                    String::from_utf8_lossy(&message.data),
                    get_node_name(&propagation_peer_id),
                    short_peer_id(&propagation_peer_id),
                    message.topic
                ));

                match message.topic.into() {
                    // When a node receives Kfrags in a broadcast/multicast
                    GossipTopic::BroadcastKfrag(agent_name_nonce, total_frags, frag_num)  => {

                        let k: KeyFragmentMessage = serde_json::from_slice(&message.data)
                            .expect("Error deserializing KeyFragmentMessage");

                        if let Some(capsule) = k.capsule {

                            let verified_kfrag = k.kfrag.verify(
                                &k.verifying_pk,
                                Some(&k.alice_pk),
                                Some(&k.bob_pk)
                            ).expect("err verifying kfrag");

                            // all nodes store cfrags locally
                            // nodes should also inform the vessel_node that they hold a fragment
                            self.peer_manager.insert_cfrags(
                                &agent_name_nonce,
                                CapsuleFragmentMessage {
                                    frag_num: k.frag_num,
                                    threshold: k.threshold,
                                    cfrag: umbral_pre::reencrypt(&capsule, verified_kfrag).unverify(),
                                    verifying_pk: k.verifying_pk,
                                    alice_pk: k.alice_pk, // vessel
                                    bob_pk: k.bob_pk, // next vessel
                                    sender_peer_id: self.peer_id,
                                    vessel_peer_id: propagation_peer_id,
                                    next_vessel_peer_id: k.next_vessel_peer_id,
                                    capsule: Some(capsule),
                                    ciphertext: k.ciphertext
                                }
                            );
                            self.print_stored_cfrags(&self.peer_manager.cfrags);

                            // Track agent vessel: current and next vessel (peer_ids)
                            println!("\n\npeer_info:");
                            for (pid, peer_info) in &self.peer_manager.peer_info {
                                println!("{} agent_vessel: {:?}", get_node_name(pid), peer_info.agent_vessel);
                            }

                            if !self.peer_manager.peer_info_contains_agent(&k.vessel_peer_id) {
                                println!("SAVING AGENT VESSEL");
                                self.peer_manager.set_peer_info_agent_vessel(
                                    &agent_name_nonce,
                                    &total_frags,
                                    k.vessel_peer_id,
                                    k.next_vessel_peer_id,
                                );
                            }

                            println!("\n\npeer_info after:");
                            for (pid, peer_info) in &self.peer_manager.peer_info {
                                println!("{} agent_vessel: {:?}", get_node_name(pid), peer_info.agent_vessel);
                            }

                            // If node is not next vessel, let vessel node know it holds a fragment
                            if self.peer_id != k.next_vessel_peer_id {
                                // request-response: let vessel know this peer received a fragment
                                let request_id = self
                                    .swarm
                                    .behaviour_mut()
                                    .request_response
                                    .send_request(
                                        &k.next_vessel_peer_id,
                                        FragmentRequestEnum::ProvidingFragment(
                                            agent_name_nonce,
                                            frag_num,
                                            self.peer_id // sender_peer_id
                                        )
                                    );
                            }
                        }

                    }
                    GossipTopic::TopicSwitch => {
                        let ts: TopicSwitch = serde_json::from_slice(&message.data)
                            .expect("Error deserializing TopicSwitch");

                        println!("Received GossipTopic::TopicSwitch");
                        let agent_name_nonce = ts.next_topic.agent_name_nonce;
                        let total_frags = ts.next_topic.total_frags;

                        NODE_SEED_NUM.with(|n| *n.borrow() % total_frags);
                        let frag_num = self.seed % total_frags;
                        // assert_eq!(frag_num, NODE_SEED_NUM.take());
                        println!("self.seed: {}", self.seed);
                        println!("total_frags: {}", total_frags);
                        println!("frag_num: {}", frag_num);

                        let topic = GossipTopic::BroadcastKfrag(agent_name_nonce, total_frags, frag_num);
                        self.subscribe_topics(&vec![topic.to_string()]);
                    }
                    // Nothing else to do for GossipTopic::Unknown
                    GossipTopic::Unknown => {
                        println!("Unknown topic: {:?}", message.data);
                    }
                }
            }
            gossipsub::Event::GossipsubNotSupported {..} => {}
            gossipsub::Event::SlowPeer {..} => {}
            gossipsub::Event::Unsubscribed { peer_id, topic } => {}
            gossipsub::Event::Subscribed { peer_id, topic } => {

                // TODO: check TEE attestation from Peer before adding peer to channel
                // We want to ensure that the Peer is running in a TEE and not subscribe to multiple
                // fragment channels for the same agent (collusion attempts)
                //
                // if peer is registered on multiple kfrag broadcasts, blacklist them
                // Peers should be on just 1 kfrag broadcast channel.
                // We can enforce this if the binary runs in a TEE

                match topic.into() {
                    GossipTopic::BroadcastKfrag(
                        agent_name_nonce,
                        total_frags,
                        frag_num
                    ) => {},
                    _ => {}
                }

            }
        }
    }

    pub(crate) async fn broadcast_topic_switch(&mut self, topic_switch: &TopicSwitch) {

        let gossip_topic_bytes = serde_json::to_vec(&topic_switch)
            .expect("serde error");

        self.log(format!("Broadcasting topic switch"));
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(
                GossipTopic::TopicSwitch,
                gossip_topic_bytes
            )
            .map_err(|e| anyhow!(e.to_string()))
            .ok();
    }

    fn print_stored_cfrags(&self, self_kfrags: &HashMap<AgentNameWithNonce, CapsuleFragmentMessage>) {
        self.log(format!("Storing CapsuleFragmentMessage:"));
        let _ = self_kfrags.iter()
            .map(|(k, v)|
                println!("key: {}\tfrag_num: {}", k, v.frag_num)
            )
            .collect::<Vec<()>>();
    }

    pub fn print_subscribed_topics(&mut self) {
        self.log(format!("Topics subscribed:"));
        let _ = self.swarm.behaviour_mut().gossipsub.topics()
            .into_iter()
            .map(|t| println!("{:?}", t.as_str())).collect::<()>();
    }

    pub fn subscribe_topics(&mut self, topic_strs: &Vec<String>) -> Vec<String> {
        topic_strs.iter()
            .filter_map(|topic_str| {
                let topic = gossipsub::IdentTopic::new(topic_str);
                println!("Subscribing to: {}", topic);
                if let Some(_) = self.swarm.behaviour_mut().gossipsub.subscribe(&topic).ok() {

                    let entry = self.topics
                            .entry(topic_str.to_string())
                            .insert_entry(topic);

                    Some(entry.key().to_string())
                } else {
                    None
                }
                // TODO: assert node subscribed to only 1 topic for receiving kfrags
            })
            .collect()
    }

    pub fn unsubscribe_topics(&mut self, topic_strs: &Vec<String>) -> Vec<String> {
        topic_strs.iter()
            .filter_map(|topic_str| {
                let topic = gossipsub::IdentTopic::new(topic_str);
                self.swarm.behaviour_mut().gossipsub.unsubscribe(&topic);
                self.topics.remove(topic_str)
                    .map(|t| t.to_string())
            })
            .collect()
    }

}

