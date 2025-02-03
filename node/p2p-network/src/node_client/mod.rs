
mod commands;
mod stdin_handlers;

pub use commands::NodeCommand;

use std::collections::{HashMap, HashSet};
use color_eyre::{Result, eyre::anyhow, eyre::Error};
use colored::Colorize;
use futures::FutureExt;
use tokio::sync::{mpsc, oneshot};
use hex;
use libp2p::{core::Multiaddr, PeerId};

use crate::get_node_name;
use crate::types::{
    ChatMessage,
    TopicSwitch,
    GossipTopic,
    NetworkLoopEvent,
    UmbralPublicKeyResponse,
    CapsuleFragmentIndexed,
};
use crate::behaviour::{
    KeyFragmentMessage
};
use umbral_pre::VerifiedCapsuleFrag;
use runtime::reencrypt::{
    UmbralKey,
    generate_pre_kfrags,
};
use runtime::llm::{test_claude_query, test_deepseek_query, AgentSecretsJson};



#[derive(Clone)]
pub struct NodeClient {
    pub peer_id: PeerId,
    pub command_sender: mpsc::Sender<NodeCommand>,
    pub node_name: String,
    pub hosting_agent_name: Option<String>,
    pub agent_secrets_json: Option<AgentSecretsJson>,

    pub chat_cmd_sender: mpsc::Sender<ChatMessage>,

    umbral_key: UmbralKey, // keep private
    umbral_capsule: Option<umbral_pre::Capsule>,
    umbral_ciphertext: Option<Box<[u8]>>,
}

impl NodeClient {

    pub fn new(
        peer_id: PeerId,
        command_sender: mpsc::Sender<NodeCommand>,
        node_name: String,
        chat_cmd_sender: mpsc::Sender<ChatMessage>,
        umbral_key: UmbralKey,
    ) -> Self {

        Self {
            command_sender: command_sender,
            peer_id: peer_id,
            node_name: node_name,
            chat_cmd_sender: chat_cmd_sender,
            hosting_agent_name: None,
            agent_secrets_json: None,
            umbral_key: umbral_key,
            umbral_capsule: None,
            umbral_ciphertext: None,
        }
    }

    pub async fn listen_to_network_events(
        &mut self,
        mut network_event_receiver: mpsc::Receiver<NetworkLoopEvent>
    ) -> Result<()> {
        loop {
            match network_event_receiver.recv().await {
                // Reply with the content of the file on incoming requests.
                Some(NetworkLoopEvent::InboundCfragRequest {
                    agent_name,
                    agent_nonce,
                    frag_num,
                    channel
                }) => {
                    // check if vessel node for the agent_name is still alive.
                    // if so, then reject request.
                    if let Some(kfrag_num) = frag_num {
                        self.command_sender
                            .send(NodeCommand::RespondCfrag {
                                agent_name,
                                agent_nonce,
                                frag_num: kfrag_num,
                                channel
                            })
                            .await
                            .expect("Command receiver not to be dropped.");
                    } else {
                        self.log(format!("frag_num missing"));
                    }
                }

                Some(NetworkLoopEvent::Respawn(agent_name, agent_nonce, prev_vessel_peer_id)) => {

                    match self.request_respawn(agent_name.clone(), agent_nonce, Some(prev_vessel_peer_id)).await {
                        Err(e) => println!("Error respawning agent in new vessel"),
                        Ok(agent_secrets_json) => {

                            let topics = self.subscribe_topics(vec![
                                GossipTopic::BroadcastKfrag(agent_name.clone(), 1, 0).to_string(),
                                GossipTopic::BroadcastKfrag(agent_name.clone(), 1, 1).to_string(),
                                GossipTopic::BroadcastKfrag(agent_name.clone(), 1, 2).to_string(),
                                GossipTopic::BroadcastKfrag(agent_name.clone(), 1, 3).to_string(),
                            ]).await;

                            let _ = self.encrypt_secret(agent_secrets_json.clone());
                            let agent_nonce = agent_secrets_json.agent_nonce;

                            // // broadcast topic switch to new Agent with new ID
                            // let topic_switch = Some(TopicSwitch {
                            //     prev_topic: agent_name.to_string(), //
                            //     next_topic: agent_name.to_string(), // +1
                            //     prev_vessel_peer_id: prev_vessel_peer_id.clone()
                            // });

                            self.broadcast_kfrags(agent_name, agent_nonce, 3, 2).await.expect("respawn err");

                            self.agent_secrets_json = Some(agent_secrets_json.clone());

                            if let Some(anthropic_api_key) = agent_secrets_json.anthropic_api_key.clone() {
                                self.log(format!("Decrypted LLM API keys, querying LLM:"));

                                let response = test_claude_query(
                                    anthropic_api_key,
                                    "What is your name and what do you do?",
                                    &agent_secrets_json.context
                                ).await.unwrap();

                                println!("\n{} {}\n", "Claude:".bright_black(), response.yellow());
                            }

                        }
                    };
                }

                Some(NetworkLoopEvent::ReBroadcastKfrags(agent_name, agent_nonce)) => {
                    self.broadcast_kfrags(agent_name, agent_nonce, 3, 2).await.expect("respawn err");
                }

                e => {
                    panic!("Error <network_event_receiver>: {:?}", e);
                }
            }
        }
    }

    pub fn log<S: std::fmt::Display>(&self, message: S) {
        println!("{} {}{} {}",
            "NodeClient".bright_blue(), self.node_name.yellow(), ">".blue(),
            message
        );
    }

    pub async fn ask_llm(&mut self, question: &str) {
        if let Some(agent_secrets_json) = &self.agent_secrets_json {
            if let Some(anthropic_api_key) = agent_secrets_json.anthropic_api_key.clone() {
                println!("Context: {}", agent_secrets_json.context.blue());
                let response = test_claude_query(
                    anthropic_api_key,
                    question,
                    &agent_secrets_json.context
                ).await.unwrap();
                println!("\n{} {}\n", "Claude:".bright_black(), response.yellow());
            }
        }
    }

    pub fn encrypt_secret(&mut self, agent_secrets: AgentSecretsJson) -> Result<()> {

        self.hosting_agent_name = Some(agent_secrets.agent_name.clone());
        let agent_secrets_bytes = &serde_json::to_vec(&agent_secrets)?;

        let (capsule, ciphertext) = umbral_pre::encrypt(
            &self.umbral_key.public_key,
            &agent_secrets_bytes
        ).unwrap();

        let plaintext_alice = umbral_pre::decrypt_original(
            &self.umbral_key.secret_key,
            &capsule,
            &ciphertext
        ).expect("Should be able to decrypt own ciphertext");

        let agent_secrets_json = serde_json::from_slice::<AgentSecretsJson>(&plaintext_alice)
            .expect("error marshalling decrypted plaintext to JSON data");

        // self.log(format!("Decryptable JSON data: {}", decrypted_data));
        self.log(format!("Encrypted AgentSecretsJson data: {:?}", hex::encode(ciphertext.clone())).black());

        self.agent_secrets_json = Some(agent_secrets_json);
        self.umbral_capsule = Some(capsule);
        self.umbral_ciphertext = Some(ciphertext);
        Ok(())
    }


    pub async fn start_listening_to_network(&mut self, listen_address: Option<Multiaddr>) -> Result<()> {

        let (sender, receiver) = oneshot::channel();
        // In case a listen address was provided use it, otherwise listen on any address.
        let addr = match listen_address {
            Some(addr) => addr,
            // None => "/ip4/0.0.0.0/tcp/0".parse().unwrap(),
            None => "/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap(),
        };

        self.command_sender
            .send(NodeCommand::StartListening { addr, sender })
            .await
            .expect("Command receiver not to be dropped.");

        match receiver.await {
            Err(e) => Err(anyhow!(e.to_string())),
            Ok(_) => Ok(())
        }
    }

    pub async fn subscribe_topics(&mut self, topics: Vec<String>) -> Result<Vec<String>> {
        let (sender, receiver) = oneshot::channel();

        println!("subscribing to: {:?}", topics);
        self.command_sender
            .send(NodeCommand::SubscribeTopics { topics, sender })
            .await
            .expect("Command receiver not to be dropped.");

        match receiver.await {
            Ok(subscribed_topics) => {
                self.log(format!("Subscribed to {:?}", subscribed_topics));
                Ok(subscribed_topics)
            }
            Err(e) => Err(e.into())
        }
    }

    pub async fn unsubscribe_topics(&mut self, topics: Vec<String>) -> Result<Vec<String>> {
        let (sender, receiver) = oneshot::channel();

        self.command_sender
            .send(NodeCommand::UnsubscribeTopics { topics, sender })
            .await
            .expect("Command receiver not to be dropped.");

        match receiver.await {
            Ok(unsubscribed_topics) => {
                self.log(format!("Unsubscribed from {:?}", unsubscribed_topics));
                Ok(unsubscribed_topics)
            }
            Err(e) => Err(e.into())
        }
    }

    pub async fn broadcast_kfrags(
        &mut self,
        agent_name: String,
        agent_nonce: usize,
        shares: usize,
        threshold: usize
    ) -> Result<UmbralPublicKeyResponse> {

        let umbral_public_keys = self.get_peer_umbral_pks(
            agent_name.clone(),
            agent_nonce,
        ).await;
        // self.log(format!("received Umbral PKs: {:?}\n", umbral_public_keys));

        // choose the next node in the queue to become the next vessel
        match umbral_public_keys.iter().next() {
            None => Err(anyhow!("No Umbral PK Peers found")),
            Some(new_vessel_pk) => {

                self.log(format!("\n\nNext Vessel: {}\n\t{:?}\n\t{}\n",
                    get_node_name(&new_vessel_pk.umbral_peer_id.clone().into()),
                    new_vessel_pk.umbral_peer_id,
                    new_vessel_pk.umbral_public_key,
                ).yellow());

                let bob_pk = new_vessel_pk.umbral_public_key;

                // Alice generates reencryption key fragments for Ursulas (MPC nodes)
                self.log(format!("Generating share fragments: ({shares},{threshold})"));
                let kfrags = generate_pre_kfrags(
                    &self.umbral_key.secret_key, // alice_sk
                    &bob_pk, // bob_pk
                    &self.umbral_key.signer, // alice
                    threshold,
                    shares
                );

                for (i, kfrag) in kfrags.into_iter().enumerate() {

                    let topic = GossipTopic::BroadcastKfrag(
                        agent_name.clone(),
                        agent_nonce, // nonce
                        i as u32
                    );

                    self.command_sender
                        .send(NodeCommand::BroadcastKfrags(
                            KeyFragmentMessage {
                                topic: topic,
                                frag_num: i,
                                threshold: threshold,
                                kfrag: kfrag,
                                verifying_pk: self.umbral_key.verifying_pk,
                                alice_pk: self.umbral_key.public_key,
                                bob_pk, // bob_pk
                                vessel_peer_id: self.peer_id,
                                next_vessel_peer_id: new_vessel_pk.umbral_peer_id.clone().into(),
                                capsule: self.umbral_capsule.clone(),
                                ciphertext: self.umbral_ciphertext.clone(),
                            }
                        )).await?;
                }

                Ok(new_vessel_pk.to_owned())
            }
        }
    }


    pub async fn get_peer_umbral_pks(&mut self, agent_name: String, agent_nonce: usize) -> Vec<UmbralPublicKeyResponse> {
        let (sender, mut receiver) = mpsc::channel(100);

        self.command_sender
            .send(NodeCommand::GetPeerUmbralPublicKey { sender, agent_name, agent_nonce })
            .await
            .expect("Command receiver not to be dropped.");

        let mut pks: Vec<UmbralPublicKeyResponse> = vec![];
        while let Some(pk) = receiver.recv().await {
            self.log(format!("Received Peer Umbral PK = {}", pk));
            pks.push(pk);
        }
        pks
    }

    pub async fn get_agent_kfrag_peers(
        &mut self,
        agent_name: String,
        agent_nonce: usize
    ) -> HashMap<u32, HashSet<PeerId>> {
        let (sender, receiver) = oneshot::channel();

        self.command_sender
            .send(NodeCommand::GetKfragBroadcastPeers {
                agent_name: agent_name,
                agent_nonce: agent_nonce,
                sender: sender
            })
            .await
            .expect("Command receiver not to be dropped.");

        receiver.await.expect("get kfrags peers not to drop")
    }

    pub async fn request_cfrags(
        &mut self,
        agent_name: String,
        agent_nonce: usize,
        opt_prev_vessel_peer_id: Option<PeerId>
    ) -> Vec<Result<Vec<u8>>> {

        let providers_hmap = self.get_agent_kfrag_peers(
            agent_name.clone(),
            agent_nonce
        ).await;

        let mut results = vec![];

        for (frag_num, peers) in providers_hmap.into_iter() {

            let peers = match opt_prev_vessel_peer_id {
                None => peers,
                Some(prev_vessel_peer_id) => {
                    peers.iter()
                        .filter_map(|&peer_id| {
                            if peer_id != prev_vessel_peer_id {
                                Some(peer_id)
                            } else {
                                None
                            }
                        })
                        .collect::<HashSet<PeerId>>()
                }
            };

            // Request key_fragment(n) from each node that holds that fragment.
            let requests = peers.iter().map(|&peer_id| {

                let agent_name2 = agent_name.clone();
                let nc = self.clone();
                self.log(format!("Requesting cfrag({}) from: {:?}", frag_num, get_node_name(&peer_id)));

                async move {
                    let (sender, receiver) = oneshot::channel();

                    nc.command_sender
                        .send(NodeCommand::RequestFile {
                            agent_name: agent_name2,
                            agent_nonce: agent_nonce,
                            frag_num: Some(frag_num as usize),
                            peer: peer_id,
                            sender
                        })
                        .await
                        .expect("Command receiver not to be dropped.");

                    receiver.await.expect("Sender not be dropped.")
                }.boxed()
            });

            if requests.len() > 0 {
                // Await the requests, ignore the remaining once a single one succeeds.
                if let Ok((cfrag_raw_bytes, _)) = futures::future::select_ok(requests).await {
                    results.push(Ok(cfrag_raw_bytes));
                }
            }
        };

        results
    }


    fn parse_cfrags(&self, cfrags_raw: Vec<Result<Vec<u8>, Error>>) -> (
        Vec<VerifiedCapsuleFrag>,
        Vec<CapsuleFragmentIndexed>,
        u32
    ) {
        let mut capsule_frags: HashMap<u32, VerifiedCapsuleFrag> = HashMap::new();
        let mut new_vessel_cfrags: Vec<CapsuleFragmentIndexed> = Vec::new();
        let mut total_frags = 0;

        for cfrag_result in cfrags_raw.iter() {
            if let Ok(cfrag_bytes) = cfrag_result {
                // println!("CFRG BYTEs: {:?}", cfrag_bytes);
                match serde_json::from_slice::<Option<CapsuleFragmentIndexed>>(&cfrag_bytes) {
                    Err(e) => panic!("{}", e.to_string()),
                    Ok(opt_cfrag) => match opt_cfrag {
                        None => {
                            println!("No cfrags found.");
                        }
                        Some(cfrag) => {
                            total_frags += 1;
                            self.log(format!("Success!: frag_num({}), total frags: {}", cfrag.frag_num, total_frags));

                            let new_vessel_pk  = cfrag.bob_pk;
                            let kfrag_num = cfrag.frag_num;
                            self.log(format!("Received Cfrag({}): {}", kfrag_num, cfrag));

                            // Bob must check that cfrags are valid
                            // assemble kfrags, verify them as cfrags.
                            let verified_cfrag = cfrag.cfrag.clone().verify(
                                &cfrag.capsule.clone().unwrap(),
                                &cfrag.verifying_pk, // verifying pk
                                &cfrag.alice_pk, // alice pk
                                &new_vessel_pk // bob pk
                            ).expect("Error verifying Cfrag");

                            self.log(format!("Verified Cfrag({}): {}", kfrag_num, verified_cfrag));

                            new_vessel_cfrags.push(cfrag);
                            capsule_frags.insert(kfrag_num as u32, verified_cfrag);
                        }
                    }
                }
            }
        }

        let verified_cfrags = capsule_frags.into_iter()
            .map(|(index, verified_cfrags)| verified_cfrags)
            .collect::<Vec<VerifiedCapsuleFrag>>();

        (verified_cfrags, new_vessel_cfrags, total_frags)
    }

    fn decrypt_cfrags(
        &self,
        verified_cfrags: Vec<VerifiedCapsuleFrag>,
        mut new_vessel_cfrags: Vec<CapsuleFragmentIndexed>,
        total_frags_received: u32
    ) -> Result<AgentSecretsJson, Error> {

        // get next vessel (can randomise as well)
        let mut new_vessel_pk = new_vessel_cfrags.pop().unwrap();
        let threshold = new_vessel_pk.threshold as usize;
        self.log(format!("Received {}/{} required CapsuleFrags", total_frags_received, threshold));

        // Bob opens the capsule by using at least `threshold` cfrags,
        // and then decrypts the re-encrypted ciphertext.
        match umbral_pre::decrypt_reencrypted(
            &self.umbral_key.secret_key, // bob
            &new_vessel_pk.alice_pk, // alice
            &new_vessel_pk.capsule.as_mut().unwrap(),
            verified_cfrags,
            new_vessel_pk.ciphertext.as_mut().unwrap()
        ) {
            Ok(plaintext_bob) => {

                let decrypted_data: serde_json::Value = serde_json::from_slice(&plaintext_bob)?;
                let agent_secrets_str = serde_json::to_string_pretty(&decrypted_data)?;
                self.log(format!("Decrypted (re-encrypted) agent data:\n{}", agent_secrets_str).yellow());

                let agent_secrets_json: AgentSecretsJson = serde_json::from_slice(&plaintext_bob)?;

                Ok(agent_secrets_json)
            },
            Err(e) => {
                let node_name = get_node_name(&self.peer_id);
                self.log(format!(">>> Err({})", e));
                if total_frags_received < threshold as u32 {
                    self.log(format!(">>> Not enough fragments. Need {threshold}, received {total_frags_received}"));
                } else {
                    self.log(format!(">>> Not decryptable by user {} with: {}", node_name, self.umbral_key.public_key));
                    self.log(format!(">>> Only decryptable by new vessel with: {}", new_vessel_pk.bob_pk));
                }
                Err(anyhow!(e.to_string()))
            }
        }
    }

    pub async fn request_respawn(
        &mut self,
        agent_name: String,
        agent_nonce: usize,
        prev_vessel_peer_id: Option<PeerId>
    ) -> Result<AgentSecretsJson> {

        let cfrags_raw = self.request_cfrags(
            agent_name,
            agent_nonce,
            prev_vessel_peer_id
        ).await;

        let (
            verified_cfrags,
            new_vessel_cfrags,
            total_frags_received
        ) = self.parse_cfrags(cfrags_raw);

        self.decrypt_cfrags(verified_cfrags, new_vessel_cfrags, total_frags_received)
    }

}
