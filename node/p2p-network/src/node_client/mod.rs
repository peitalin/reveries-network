mod commands;
mod network_events_listener;
mod reincarnation;
pub(crate) mod container_manager;

pub use commands::NodeCommand;
pub use container_manager::{ContainerManager, RestartReason};

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use color_eyre::{Result, eyre::anyhow, eyre::Error};
use colored::Colorize;
use futures::FutureExt;
use hex;
use libp2p::{core::Multiaddr, PeerId};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, debug, error, warn};

use crate::{get_node_name, short_peer_id, TryPeerId};
use crate::network_events::peer_manager::peer_info::AgentVesselInfo;
use crate::network_events::NodeIdentity;
use crate::types::{
    AgentNameWithNonce,
    CapsuleFragmentMessage,
    GossipTopic,
    KeyFragmentMessage,
    ReverieKeyfragMessage,
    NetworkEvent,
    NextTopic,
    PrevTopic,
    RespawnId,
    TopicSwitch,
    NodeVesselWithStatus,
    VesselStatus,
    Reverie,
    ReverieType,
    ReverieId,
    ReverieKeyfrag,
    ReverieMessage,
};
use crate::SendError;
use crate::behaviour::heartbeat_behaviour::TeePayloadOutEvent;
use runtime::reencrypt::{UmbralKey, VerifiedCapsuleFrag};
use runtime::llm::{test_claude_query, AgentSecretsJson};



#[derive(Clone)]
pub struct NodeClient<'a> {
    pub node_id: NodeIdentity<'a>,
    reveries: HashMap<String, Reverie>,
    pub command_sender: mpsc::Sender<NodeCommand>,
    // container app state
    container_manager: Arc<tokio::sync::RwLock<ContainerManager>>,
    // hb subscriptions for rpc clients
    pub heartbeat_receiver: async_channel::Receiver<TeePayloadOutEvent>,
    // keep private in TEE
    agent_secrets_json: Option<AgentSecretsJson>,
    umbral_key: UmbralKey,
}

impl<'a> NodeClient<'a> {
    pub fn new(
        node_id: NodeIdentity<'a>,
        command_sender: mpsc::Sender<NodeCommand>,
        umbral_key: UmbralKey,
        container_manager: Arc<tokio::sync::RwLock<ContainerManager>>,
        heartbeat_receiver: async_channel::Receiver<TeePayloadOutEvent>,
    ) -> Self {
        Self {
            node_id: node_id,
            reveries: HashMap::new(),
            command_sender: command_sender,
            // manages container reboot
            container_manager: container_manager,
            // hb subscriptions for rpc clients
            heartbeat_receiver: heartbeat_receiver,
            // keep private in TEE
            agent_secrets_json: None,
            umbral_key: umbral_key,
        }
    }

    pub async fn start_listening_to_network(&mut self, listen_address: Option<Multiaddr>) -> Result<()> {
        let (sender, receiver) = oneshot::channel();
        // In case a listen address was provided use it, otherwise listen on any address.
        let addr = match listen_address {
            Some(addr) => addr,
            None => "/ip4/0.0.0.0/tcp/0".parse()?,
        };

        self.command_sender
            .send(NodeCommand::StartListening { addr, sender })
            .await?;

        receiver.await?.map_err(|e| SendError(e.to_string()).into())
    }

    pub async fn subscribe_topics(&self, topics: Vec<String>) -> Result<Vec<String>> {
        let (sender, receiver) = oneshot::channel();

        self.command_sender
            .send(NodeCommand::SubscribeTopics {
                topics,
                sender
            })
            .await
            .expect("Command receiver not to be dropped.");

        receiver.await.map_err(|e| e.into())
    }

    pub async fn unsubscribe_topics(&self, topics: Vec<String>) -> Result<Vec<String>> {
        let (sender, receiver) = oneshot::channel();

        self.command_sender
            .send(NodeCommand::UnsubscribeTopics {
                topics,
                sender
            })
            .await
            .expect("Command receiver not to be dropped.");

        receiver.await.map_err(|e| e.into())
    }

    pub fn make_reverie_horcruxes(
        &self,
        reverie: &Reverie,
        target_vessel: NodeVesselWithStatus,
    ) -> Result<Vec<ReverieKeyfrag>> {
        let bob_pk = target_vessel.umbral_public_key;
        // Alice generates reencryption key fragments for Ursulas (MPC nodes)
        info!("Generating fragments for new vessel: {}-of-{} key", reverie.threshold, reverie.total_frags);
        let kfrags = self.umbral_key.generate_pre_keyfrags(
            &bob_pk, // bob_pk
            reverie.threshold,
            reverie.total_frags
        ).iter().enumerate().map(|(i, kfrag)| {
            ReverieKeyfrag {
                id: reverie.id.clone(),
                reverie_type: reverie.reverie_type.clone(),
                frag_num: i,
                threshold: reverie.threshold,
                total_frags: reverie.total_frags,
                umbral_keyfrag: serde_json::to_vec(&kfrag).expect(""),
                umbral_capsule: reverie.umbral_capsule.clone(),
                alice_pk: self.umbral_key.public_key,
                bob_pk: target_vessel.umbral_public_key,
                verifying_pk: self.umbral_key.verifying_pk,
            }
        }).collect::<Vec<ReverieKeyfrag>>();
        Ok(kfrags)
    }

    pub fn create_reverie(&mut self, agent_secrets: AgentSecretsJson, threshold: usize, total_frags: usize) -> Result<Reverie> {

        let (
            capsule,
            ciphertext
        ) = self.umbral_key.encrypt_bytes(&serde_json::to_vec(&agent_secrets)?)?;

        // check agent_secrets are decryptable and parsable
        let agent_secrets_json: AgentSecretsJson = serde_json::from_slice(
            &self.umbral_key.decrypt_original(
                &capsule,
                &ciphertext
            )?
        )?;

        info!("{}{}", "Encrypted AgentSecretsJson:\n",
            format!("{}", hex::encode(ciphertext.clone())).black()
        );

        self.agent_secrets_json = Some(agent_secrets_json);
        // println!("agent_secrets_json {:?}", self.agent_secrets_json);

        let reverie = Reverie::new(
            "agent_secrets_json".to_string(),
            ReverieType::Memory,
            threshold,
            total_frags,
            capsule,
            ciphertext
        );

        println!("reverie_id {:?}", reverie.id);
        self.reveries.insert(reverie.id.clone(), reverie.clone());
        Ok(reverie)
    }

    /// Client sends a memory over TLS or some secure channel.
    /// Node encrypts with PRE and broadcasts fragments to the network
    // pub async fn broadcast_reverie(
    pub async fn broadcast_kfrags2(
        &mut self,
        // memory: AgentSecretsJson,
        reverie_id: String,
        agent_name_nonce: AgentNameWithNonce,
    ) -> Result<Reverie> {

        // Encrypt using PRE
        // let reverie = self.create_reverie(memory.clone())?;
        let reverie = match self.reveries.get(&reverie_id) {
            Some(reverie) => reverie,
            None => return Err(anyhow!("Reverie not found"))
        };

        let peer_nodes = self.get_node_vessels().await
            .into_iter()
            .filter(|v| v.vessel_status == VesselStatus::EmptyVessel)
            .collect::<Vec<NodeVesselWithStatus>>();

        let n_before = peer_nodes.len();
        let (
            target_vessel,
            remaining_peers
        ) = peer_nodes
            .split_first()
            .ok_or(anyhow!("No empty vessels found."))?;

        let n_after = peer_nodes.len();
        info!("{}", format!("Before: {n_before}, After: {n_after}"));
        assert!(n_after == n_before - 1);

        let umbral_capsule = reverie.umbral_capsule.clone();
        let umbral_ciphertext = reverie.umbral_ciphertext.clone();

        // Split into fragments
        let kfrags = self.make_reverie_horcruxes(
            &reverie,
            target_vessel.clone()
        )?;

        // Send Kfrags to MPC nodes
        for (i, kfrag) in kfrags.into_iter().enumerate() {

            let fragment_provider_peer_id = remaining_peers[i].peer_id;

            self.command_sender.send(
                NodeCommand::SendKfrag(
                    fragment_provider_peer_id,
                    ReverieKeyfragMessage {
                        reverie_keyfrag: kfrag,
                        source_peer_id: self.node_id.peer_id,
                        target_peer_id: target_vessel.peer_id,
                    },
                    Some(agent_name_nonce.clone())
                )
            ).await?;
        }

        // Send Ciphertext to target vessel
        self.command_sender.send(
            NodeCommand::SendReverie(
                target_vessel.peer_id, // Ciphertext Holder
                ReverieMessage {
                    reverie: reverie.clone(),
                    source_peer_id: self.node_id.peer_id,
                    target_peer_id: target_vessel.peer_id,
                },
                Some(agent_name_nonce)
            )
        ).await?;

        Ok(reverie.clone())
    }


    pub async fn broadcast_kfrags(
        &mut self,
        reverie_id: String,
        agent_name_nonce: AgentNameWithNonce,
        threshold: usize,
        total_frags: usize,
        target_vessel: NodeVesselWithStatus,
    ) -> Result<NodeVesselWithStatus> {

        let reverie = match self.reveries.get(&reverie_id) {
            Some(reverie) => reverie,
            None => return Err(anyhow!("Reverie not found"))
        };

        let umbral_capsule: umbral_pre::Capsule = serde_json::from_slice(&reverie.umbral_capsule).expect("");
        let umbral_ciphertext = reverie.umbral_ciphertext.clone();

        let kfrags = self.make_reverie_horcruxes(
            reverie,
            target_vessel.clone()
        )?;

        /////////////////// Gossip Broadcast ///////////////////////

        // first check that the broadcaster is subscribed to all fragment channels for the agent
        let topics = (0..total_frags)
            .map(|n| GossipTopic::BroadcastKfrag(agent_name_nonce.clone(), total_frags, n).into())
            .collect::<Vec<String>>();

        // Subscribe to broadcast kfrag topics temporarily
        self.subscribe_topics(topics.clone()).await.ok();

        for (i, kfrag) in kfrags.into_iter().enumerate() {
            let topic = GossipTopic::BroadcastKfrag(
                agent_name_nonce.clone(),
                total_frags,
                i
            );
            self.command_sender.send(
                NodeCommand::BroadcastKfrags(KeyFragmentMessage {
                    topic: topic,
                    reverie_id: reverie_id.clone(),
                    frag_num: i,
                    threshold: threshold,
                    total_frags: total_frags,
                    kfrag: serde_json::from_slice(&kfrag.umbral_keyfrag).expect(""),
                    verifying_pk: self.umbral_key.verifying_pk,
                    alice_pk: self.umbral_key.public_key,
                    bob_pk: target_vessel.umbral_public_key, // bob_pk
                    vessel_peer_id: self.node_id.peer_id,
                    next_vessel_peer_id: target_vessel.peer_id,
                    capsule: umbral_capsule.clone(),
                    ciphertext: umbral_ciphertext.clone(),
                })
            ).await?;
        }

        info!("{}", format!(
            "Next Vessel: {} {} {}",
            get_node_name(&target_vessel.peer_id),
            short_peer_id(target_vessel.peer_id),
            target_vessel.umbral_public_key,
        ).yellow());

        // unsubscribe from the kfrag broadcast topics
        self.unsubscribe_topics(topics).await.ok();
        Ok(target_vessel.to_owned())
    }

    pub async fn get_next_vessel(&mut self) -> Result<NodeVesselWithStatus> {
        match self.get_node_vessels().await.iter()
            .filter(|v| v.vessel_status == VesselStatus::EmptyVessel)
            .next()
        {
            None => Err(anyhow!("No empty vessels found.")),
            Some(next_vessel) => Ok(next_vessel.clone())
        }
    }

    pub async fn get_node_vessels(&self) -> Vec<NodeVesselWithStatus> {
        let (sender, mut receiver) = mpsc::channel(100);
        self.command_sender
            .send(NodeCommand::GetNodeVesselStatusesFromKademlia { sender })
            .await
            .expect("Command receiver not to be dropped.");

        let mut pks: Vec<NodeVesselWithStatus> = vec![];
        while let Some(pk) = receiver.recv().await {
            debug!("Received Peer Umbral PK => {}", pk);
            pks.push(pk);
        }
        pks
    }

    pub async fn get_kfrag_providers(
        &mut self,
        reverie_id: ReverieId,
    ) -> HashMap<usize, HashSet<PeerId>> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(NodeCommand::GetKfragProviders {
                reverie_id: reverie_id,
                sender: sender
            })
            .await
            .expect("Command receiver not to be dropped.");

        receiver.await.expect("get kfrags providers not to drop")
    }

    pub async fn request_cfrags(
        &mut self,
        reverie_id: ReverieId,
        current_vessel_peer_id: PeerId
    ) -> Vec<Result<Vec<u8>>> {

        let providers_hmap = self.get_kfrag_providers(reverie_id.clone()).await;

        info!("Finding providers for: {}", reverie_id);
        info!("filter out vessel peer: {:?}", current_vessel_peer_id);
        info!("providers HashMap<frag_num, peers>: {:?}\n", providers_hmap);

        let mut results = vec![];

        for (frag_num, peers) in providers_hmap.into_iter() {

            let peers = peers.iter()
                .filter_map(|&peer_id| {
                    if peer_id != current_vessel_peer_id {
                        Some(peer_id)
                    } else {
                        None
                    }
                })
                .collect::<HashSet<PeerId>>();

            // Request key_fragment(n) from each node that holds that fragment.
            let requests = peers.iter().map(|&peer_id| {
                info!("Requesting {} cfrag({}) from {}", &reverie_id, &frag_num, get_node_name(&peer_id));
                let nc = self.clone();
                let reverie_id = reverie_id.clone();
                async move {
                    let (sender, receiver) = oneshot::channel();
                    nc.command_sender
                        .send(NodeCommand::RequestCapsuleFragment {
                            reverie_id,
                            frag_num: frag_num,
                            peer_id: peer_id,
                            sender
                        })
                        .await
                        .expect("Command receiver not to be dropped.");

                    receiver.await.expect("Sender not be dropped.")
                }.boxed()
            });

            // Await the requests, ignore the remaining once a single one succeeds.
            if let Ok((cfrag_raw_bytes, _)) = futures::future::select_ok(requests).await {
                results.push(Ok(cfrag_raw_bytes));
            }
        };

        results
    }

    fn parse_cfrags(&self, cfrags_raw: Vec<Result<Vec<u8>, Error>>) -> (
        Vec<VerifiedCapsuleFrag>,
        Vec<CapsuleFragmentMessage>,
        u32
    ) {
        let mut capsule_frags: HashMap<u32, VerifiedCapsuleFrag> = HashMap::new();
        let mut new_vessel_cfrags: Vec<CapsuleFragmentMessage> = Vec::new();
        let mut total_frags = 0;

        for cfrag_result in cfrags_raw.iter() {
            if let Ok(cfrag_bytes) = cfrag_result {
                match serde_json::from_slice::<Option<CapsuleFragmentMessage>>(&cfrag_bytes) {
                    Err(e) => panic!("{}", e.to_string()),
                    Ok(opt_cfrag) => match opt_cfrag {
                        None => warn!("No cfrags found."),
                        Some(cfrag) => {

                            total_frags += 1;

                            let new_vessel_pk  = cfrag.bob_pk;
                            let kfrag_num = cfrag.frag_num;

                            info!("Success! cfrag({}) from {}\ntotal frags: {}",
                                // get_node_name(&cfrag.vessel_peer_id),
                                cfrag.frag_num,
                                get_node_name(&cfrag.kfrag_provider_peer_id),
                                total_frags
                            );

                            // Bob must check that cfrags are valid
                            // assemble kfrags, verify them as cfrags.
                            let verified_cfrag = cfrag.cfrag.clone().verify(
                                &cfrag.capsule.clone(),
                                &cfrag.verifying_pk, // verifying pk
                                &cfrag.alice_pk, // alice pk
                                &new_vessel_pk // bob pk
                            ).expect("Error verifying Cfrag");

                            new_vessel_cfrags.push(cfrag);
                            capsule_frags.insert(kfrag_num as u32, verified_cfrag);
                        }
                    }
                }
            }
        }

        let verified_cfrags = capsule_frags.into_iter()
            .map(|(_, verified_cfrags)| verified_cfrags)
            .collect::<Vec<VerifiedCapsuleFrag>>();

        (verified_cfrags, new_vessel_cfrags, total_frags)
    }

    fn decrypt_cfrags(
        &self,
        verified_cfrags: Vec<VerifiedCapsuleFrag>,
        mut new_vessel_cfrags: Vec<CapsuleFragmentMessage>,
        total_frags_received: u32
    ) -> Result<AgentSecretsJson, Error> {

        // get next vessel (can randomise as well)
        let new_vessel_pk = match new_vessel_cfrags.pop() {
            Some(vessel) => vessel,
            None => return Err(anyhow!("No CapsuleFragments found"))
        };

        let threshold = new_vessel_pk.threshold as usize;
        info!("Received {}/{} required CapsuleFrags", total_frags_received, threshold);

        // Bob (next vessel) uses his umbral_key to open the capsule by using at
        // least `threshold` cfrags, then decrypts the re-encrypted ciphertext.
        match self.umbral_key.decrypt_reencrypted(
            &new_vessel_pk.alice_pk, // delegator_pubkey
            &new_vessel_pk.capsule, // capsule,
            verified_cfrags, // verified capsule fragments
            new_vessel_pk.ciphertext
        ) {
            Ok(plaintext_bob) => {
                let decrypted_data: serde_json::Value = serde_json::from_slice(&plaintext_bob)?;
                let agent_secrets_str = serde_json::to_string_pretty(&decrypted_data)?;
                info!("{}", format!("Decrypted (re-encrypted) agent data:\n{}", agent_secrets_str).yellow());
                let agent_secrets_json: AgentSecretsJson = serde_json::from_slice(&plaintext_bob)?;
                Ok(agent_secrets_json)
            },
            Err(e) => {
                error!("{}", e);
                if total_frags_received < threshold as u32 {
                    warn!(">>> Not enough fragments. Need {threshold}, received {total_frags_received}");
                } else {
                    warn!(">>> Not decryptable by user {} with: {}", self.node_id.node_name, self.umbral_key.public_key);
                    warn!(">>> Decryptable intended for new vessel with: {}", new_vessel_pk.bob_pk);
                }
                Err(anyhow!(e.to_string()))
            }
        }
    }

    fn nname(&self) -> String {
        format!("{}{}", self.node_id.node_name.yellow(), ">".blue())
    }

    pub async fn ask_llm(&mut self, question: &str) {
        if let Some(agent_secrets_json) = &self.agent_secrets_json {
            if let Some(anthropic_api_key) = agent_secrets_json.anthropic_api_key.clone() {
                info!("Context: {}", agent_secrets_json.context.blue());
                let response = test_claude_query(
                    anthropic_api_key,
                    question,
                    &agent_secrets_json.context
                ).await.unwrap();
                info!("\n{} {}\n", "Claude:".bright_black(), response.yellow());
            }
        }
    }

    pub async fn simulate_node_failure(&mut self) -> Result<RestartReason> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(NodeCommand::SimulateNodeFailure {
            sender,
            reason: RestartReason::NetworkHeartbeatFailure,
        }).await.ok();

        receiver.await.map_err(|e| anyhow!(e.to_string()))
    }

    pub async fn get_node_state(&self) -> Result<serde_json::Value> {

        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(NodeCommand::GetNodeState {
            sender: sender,
        }).await.ok();

        let node_info = receiver.await
            .map_err(|e| anyhow!(e.to_string()))?;

        Ok(node_info)
    }

    pub fn get_hb_channel(&self) -> async_channel::Receiver<TeePayloadOutEvent> {
        self.heartbeat_receiver.clone()
    }
}

