
use std::collections::{HashMap};
use anyhow::{Result, anyhow};
use libp2p::{
    gossipsub,
    gossipsub::IdentTopic
};
use crate::behaviour::{
    CapsuleFragmentIndexed, KeyFragmentIndexed, KfragsBroadcastMessage, KfragsTopic
};
use super::EventLoop;



impl EventLoop {

    pub async fn subscribe_topic(&mut self, topic_str: &str) -> Result<IdentTopic> {
        let topic = gossipsub::IdentTopic::new(topic_str);
        self.swarm.behaviour_mut().gossipsub.subscribe(&topic)?;
        self.topics.insert(topic_str.to_string(), topic.clone());
        // assert that node is subscribed to only 1 MPC quorum/topic for receiving kfrags
        Ok(topic)
    }

    pub async fn unsubscribe_topic(&mut self, topic_str: &str) -> Result<IdentTopic> {
        let topic = gossipsub::IdentTopic::new(topic_str);
        self.swarm.behaviour_mut().gossipsub.unsubscribe(&topic)?;
        self.topics.remove(topic_str).ok_or(anyhow!("remove topic err"))
    }

    pub(super) async fn request_kfrags(&mut self, message: KfragsBroadcastMessage) -> Result<()> {

        self.log(format!("Requesting KeyFrag {:?}\n", message.topic));
        let match_topic = message.topic.to_string();
        println!("match_topic: {}", match_topic);

        match self.topics.get(&message.topic.to_string()) {
            Some(topic) => {

                let message_id = self.swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic.clone(), vec![])
                    .map_err(|e| anyhow!(e.to_string()))?;

                Ok(())
            }
            None => Err(anyhow!("Err: topic does not exist: {}", match_topic))
        }
    }

    pub(super) async fn broadcast_kfrag(&mut self, message: KfragsBroadcastMessage) {

        self.log(format!("Broadcasting KeyFrag topic: {}", message.topic));
        let match_topic = message.topic.to_string();

        let kfrag_indexed: KeyFragmentIndexed = message.into();
        let kfrag_indexed_bytes = serde_json::to_vec(&kfrag_indexed)
            .expect("serde_json::to_vec(kfrag_indexed) error");

        match self.topics.get(&match_topic) {
            Some(topic) => {
                let _ = self.swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic.clone(), kfrag_indexed_bytes)
                    .map_err(|e| anyhow!(e.to_string()));
            }
            None => {
                self.log(format!("Err: invalid topic does not exist: {}", match_topic));
                self.print_subscribed_topics();
                // Err(anyhow!("Topic does not exist".to_string()))
            }
        }

    }

    // When receiving a Gossipsub event/message
    pub async fn handle_gossipsub_event(&mut self, gevent: gossipsub::Event) {
        match gevent {
            gossipsub::Event::Message {
                propagation_source: peer_id,
                message_id,
                message,
            } => {

                self.log(format!(
                    "\n\tReceived message: '{}'\n\tid: {message_id}\n\tpeer: {peer_id}\n\ttopic: '{}'\n",
                    String::from_utf8_lossy(&message.data),
                    message.topic
                ));

                match message.topic.clone().into() {
                    KfragsTopic::Kfrag(agent_name, frag_num)  => {

                        let k: KeyFragmentIndexed = serde_json::from_slice(&message.data)
                            .expect("Error deserializing KeyFragmentIndexed");

                        if let Some(capsule) = k.capsule {

                            let verified_kfrag = k.kfrag.verify(
                                &k.verifying_pk,
                                Some(&k.alice_pk),
                                Some(&k.bob_pk)
                            ).expect("err verifying kfrag");

                            let cfrag0 = umbral_pre::reencrypt(
                                &capsule,
                                verified_kfrag
                            ).unverify();

                            self.cfrags.insert(
                                agent_name,
                                CapsuleFragmentIndexed {
                                    frag_num: k.frag_num,
                                    threshold: k.threshold,
                                    cfrag: cfrag0,
                                    verifying_pk: k.verifying_pk,
                                    alice_pk: k.alice_pk,
                                    bob_pk: k.bob_pk,
                                    capsule: Some(capsule),
                                    ciphertext: k.ciphertext
                                }
                            );
                            self.print_stored_cfrags(&self.cfrags);
                        }

                    }
                    // ignore other messages
                    KfragsTopic::RequestCfrags(_) => {}
                    KfragsTopic::Unknown(_) => {}
                }
            }
            gossipsub::Event::Unsubscribed { peer_id, topic } => {
                match topic.clone().into() {
                    KfragsTopic::Kfrag(_, _) => self.peer_manager.remove_umbral_kfrag_peer(peer_id),
                    _ => {}
                }
            }
            gossipsub::Event::GossipsubNotSupported {..} => {}
            gossipsub::Event::Subscribed { peer_id, topic } => {

                // if peer is registered on multiple kfrag broadcasts, blacklist them
                // Peers should be on just 1 kfrag broadcast channel.
                // We can enforce this if the binary runs in a TEE

                // self.log(format!("Gossipsub Peer Subscribed {:?} {:?}", peer_id, topic));
                match topic.clone().into() {
                    KfragsTopic::Kfrag(agent_name, frag_num) => {
                        self.log(format!("Adding Peer to kfrags_peers({}, {}, {})", agent_name, frag_num, peer_id));
                        self.peer_manager.insert_umbral_kfrag_peer(peer_id, agent_name, frag_num);
                    },
                    _ => {}
                }

            }
        }
    }

    fn print_stored_cfrags(&self, self_kfrags: &HashMap<String, CapsuleFragmentIndexed>) {
        self.log(format!("Storing CapsuleFragmentIndexed:"));
        let _ = self_kfrags.iter()
            .map(|(k, v)| println!("{}: {:?}", k, v.frag_num))
            .collect::<Vec<()>>();
    }

    pub fn print_subscribed_topics(&mut self) {
        self.log(format!("Topics subscribed:"));
        let _ = self.swarm.behaviour_mut().gossipsub.topics()
            .into_iter()
            .map(|t| println!("{:?}", t.as_str())).collect::<()>();
    }

}

