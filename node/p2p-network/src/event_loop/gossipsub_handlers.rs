
use std::collections::HashMap;
use color_eyre::eyre::anyhow;
use colored::Colorize;
use libp2p::gossipsub;
use crate::{get_node_name, short_peer_id};
use crate::types::{
    CapsuleFragmentIndexed,
    KeyFragmentMessage,
    GossipTopic,
    TopicSwitch,
};
use crate::create_network::NODE_SEED_NUM;
use super::EventLoop;



impl EventLoop {

    // When receiving a Gossipsub event/message
    pub async fn handle_gossipsub_event(&mut self, gevent: gossipsub::Event) {
        match gevent {
            gossipsub::Event::Message {
                propagation_source: peer_id,
                message_id,
                message,
            } => {

                self.log(format!(
                    "\n\tReceived message: '{}'\n\t|from peer: {} {}\n\t|topic: '{}'\n",
                    String::from_utf8_lossy(&message.data),
                    get_node_name(&peer_id),
                    short_peer_id(&peer_id),
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

                            self.peer_manager.set_peer_info_agent_vessel(
                                &agent_name_nonce,
                                &total_frags,
                                k.vessel_peer_id,
                                k.next_vessel_peer_id,
                            );

                            self.peer_manager.insert_peer_agent_fragments(
                                &peer_id,
                                &agent_name_nonce,
                                frag_num
                            );

                            self.peer_manager.insert_cfrags(
                                &agent_name_nonce,
                                CapsuleFragmentIndexed {
                                    frag_num: k.frag_num,
                                    threshold: k.threshold,
                                    cfrag: umbral_pre::reencrypt(&capsule, verified_kfrag).unverify(),
                                    verifying_pk: k.verifying_pk,
                                    alice_pk: k.alice_pk,
                                    bob_pk: k.bob_pk,
                                    sender_peer_id: self.peer_id,
                                    vessel_peer_id: peer_id,
                                    capsule: Some(capsule),
                                    ciphertext: k.ciphertext
                                }
                            );
                            self.print_stored_cfrags(&self.peer_manager.cfrags);

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
            gossipsub::Event::Unsubscribed { peer_id, topic } => {
                match topic.clone().into() {
                    GossipTopic::BroadcastKfrag(_, _, _) => {
                        self.peer_manager.remove_kfrags_peer(&peer_id);
                    }
                    _ => {}
                }
            }
            gossipsub::Event::Subscribed { peer_id, topic } => {

                // TODO: check TEE attestation from Peer before adding peer to channel
                // We want to ensure that the Peer is runnign in a TEE and not subscribe to multiple
                // fragment channels for the same agent (collution attempts)
                //
                // if peer is registered on multiple kfrag broadcasts, blacklist them
                // Peers should be on just 1 kfrag broadcast channel.
                // We can enforce this if the binary runs in a TEE

                self.log(format!("Gossipsub Subscribed {} to '{}'", get_node_name(&peer_id).yellow(), topic));
                // pop topic, then send back confirmation that frag_num
                match topic.clone().into() {
                    GossipTopic::BroadcastKfrag(agent_name_nonce, total_frags, frag_num) => {
                        self.log(format!(">>> Adding peer to kfrags_peers({}, {}, {})", agent_name_nonce, frag_num, short_peer_id(&peer_id)));
                        self.peer_manager.insert_kfrags_broadcast_peer(peer_id, agent_name_nonce, frag_num);
                        // self.log(format!("peers_to_agent_frags: {:?}", self.peer_manager.peers_to_agent_frags).green());
                    },
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

    fn print_stored_cfrags(&self, self_kfrags: &HashMap<String, CapsuleFragmentIndexed>) {
        self.log(format!("Storing CapsuleFragmentIndexed:"));
        let _ = self_kfrags.iter()
            .map(|(k, v)| println!("key: {}\tfrag_num: {}", k, v.frag_num))
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
                //// TODO: assert node subscribed to only 1 topic for receiving kfrags
            })
            .collect()
    }

    pub fn unsubscribe_topics(&mut self, topic_strs: &Vec<String>) -> Vec<String> {
        topic_strs.iter()
            .filter_map(|topic_str| {
                let topic = gossipsub::IdentTopic::new(topic_str);
                if let Some(_) = self.swarm.behaviour_mut().gossipsub.unsubscribe(&topic).ok() {
                    self.topics.remove(topic_str)
                        .map(|t| t.to_string())
                } else {
                    eprintln!("Could not unsubscribe from topic: {}", topic_str);
                    None
                }
            })
            .collect()
    }

}

