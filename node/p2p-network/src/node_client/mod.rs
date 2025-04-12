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
use rand::seq::SliceRandom;
use rand::thread_rng;
use sha3::{Digest, Keccak256};

use crate::{get_node_name, short_peer_id, TryPeerId};
use crate::network_events::NodeIdentity;
use crate::types::{
    ReverieNameWithNonce,
    NetworkEvent,
    NodeVesselWithStatus,
    RespawnId,
    Reverie,
    ReverieCapsulefrag,
    ReverieId,
    ReverieKeyfrag,
    ReverieKeyfragMessage,
    ReverieMessage,
    ReverieType,
    VesselStatus,
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
                source_pubkey: self.umbral_key.public_key,
                target_pubkey: target_vessel.umbral_public_key,
                source_verifying_pubkey: self.umbral_key.verifying_pk,
                target_verifying_pubkey: target_vessel.verifying_pk,
            }
        }).collect::<Vec<ReverieKeyfrag>>();
        Ok(kfrags)
    }

    pub fn create_reverie(
        &mut self,
        agent_secrets: AgentSecretsJson,  // TODO: generalize to generic
        reverie_type: ReverieType,
        threshold: usize,
        total_frags: usize
    ) -> Result<Reverie> {

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
            "agent_secrets_json description".to_string(),
            reverie_type,
            threshold,
            total_frags,
            capsule,
            ciphertext
        );

        self.reveries.insert(reverie.id.clone(), reverie.clone());
        Ok(reverie)
    }

    /// Client sends a secret/memory over TLS channel.
    /// Node encrypts with PRE and broadcasts fragments to the network
    pub async fn broadcast_reverie_keyfrags(
        &mut self,
        reverie_id: String,
    ) -> Result<(Reverie, NodeVesselWithStatus)> {

        let reverie = match self.reveries.get(&reverie_id) {
            Some(reverie) => reverie,
            None => return Err(anyhow!("Reverie not found"))
        };

        let peer_nodes = self.get_node_vessels(false).await
            .into_iter()
            .filter(|v| v.vessel_status == VesselStatus::EmptyVessel)
            .collect::<Vec<NodeVesselWithStatus>>();

        let (
            target_vessel,
            target_kfrag_providers
        ) = peer_nodes
            .split_first()
            .ok_or(anyhow!("No Peers found."))?;

        if target_kfrag_providers.contains(&target_vessel) {
            return Err(anyhow!("Target vessel cannot also be a kfrag provider"));
        }
        if target_kfrag_providers.len() < reverie.total_frags {
            return Err(anyhow!("Not connected to enough peers, need: {}, got: {}", reverie.total_frags + 1, target_kfrag_providers.len() + 1));
        }
        info!("Kfrag providers: {}", target_kfrag_providers.len());
        info!("Total frags: {}", reverie.total_frags);

        let umbral_capsule = reverie.umbral_capsule.clone();
        let umbral_ciphertext = reverie.umbral_ciphertext.clone();

        // Split into fragments
        let kfrags = self.make_reverie_horcruxes(
            &reverie,
            target_vessel.clone()
        )?;

        // Send Kfrags to peer nodes
        for (i, reverie_keyfrag) in kfrags.into_iter().enumerate() {
            let keyfrag_provider = target_kfrag_providers[i].peer_id;
            self.command_sender.send(
                NodeCommand::SendReverieKeyfrag {
                    keyfrag_provider: keyfrag_provider,
                    reverie_keyfrag_msg: ReverieKeyfragMessage {
                        reverie_keyfrag: reverie_keyfrag,
                        source_peer_id: self.node_id.peer_id,
                        target_peer_id: target_vessel.peer_id,
                    },
                }
            ).await?;
        }

        // Send Ciphertext to target vessel
        // TODO: or should Reveries be stored on DHT for easier access?
        self.command_sender.send(
            NodeCommand::SendReverie {
                ciphertext_holder: target_vessel.peer_id, // Ciphertext Holder
                reverie_msg: ReverieMessage {
                    reverie: reverie.clone(),
                    source_peer_id: self.node_id.peer_id,
                    target_peer_id: target_vessel.peer_id,
                },
            }
        ).await?;

        Ok((reverie.clone(), target_vessel.clone()))
    }

    pub async fn get_next_vessel(&mut self) -> Result<NodeVesselWithStatus> {
        match self.get_node_vessels(false).await.iter()
            .filter(|v| v.vessel_status == VesselStatus::EmptyVessel)
            .next()
        {
            None => Err(anyhow!("No empty vessels found.")),
            Some(next_vessel) => Ok(next_vessel.clone())
        }
    }

    pub async fn get_node_vessels(&self, shuffle: bool) -> Vec<NodeVesselWithStatus> {
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

        if shuffle {
            let mut rng = thread_rng();
            pks.shuffle(&mut rng);
        }

        pks
    }

    pub async fn get_kfrag_providers(
        &mut self,
        reverie_id: ReverieId,
    ) -> HashSet<PeerId> {
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

    pub async fn get_reverie(&self, reverie_id: ReverieId) -> Result<ReverieMessage> {
        let (sender, receiver) = oneshot::channel();

        self.command_sender
            .send(NodeCommand::GetReverie {
                reverie_id: reverie_id,
                sender: sender
            })
            .await?;

        receiver.await.map_err(SendError::from)?
    }

    pub async fn request_cfrags(
        &mut self,
        reverie_id: ReverieId,
        prev_failed_vessel_peer_id: PeerId
    ) -> Vec<Result<Vec<u8>, SendError>> {

        let providers = self.get_kfrag_providers(reverie_id.clone()).await;

        info!("Finding kfrag providers for: {}", reverie_id);
        info!("Filter out vessel peer: {:?}", prev_failed_vessel_peer_id);
        info!("Kfrag providers: {:?}\n", providers);

        // target vessel creates signature by signing the digest hash of reverie_id
        let digest = Keccak256::digest(reverie_id.clone().as_bytes());

        // Sign with our umbral signer key (corresponds to the verifying key).
        // The fragment provider verifies this signature with the (target) verifying key.
        // That way only the intended recipient can request the cfrags from fragment providers
        let signature = self.umbral_key.sign(&digest);

        let requests = providers.iter()
            .map(|kfrag_provider_peer_id| {

                let reverie_id2 = reverie_id.clone();
                let signature2 = signature.clone();
                let nc = self.clone();

                info!("Requesting {} cfrag from {}", &reverie_id2, get_node_name(&kfrag_provider_peer_id));
                // Request key fragment from each node that holds that fragment.
                async move {
                    let (sender, receiver) = oneshot::channel();
                    nc.command_sender
                        .send(NodeCommand::RequestCapsuleFragment {
                            reverie_id: reverie_id2.clone(),
                            kfrag_provider_peer_id: *kfrag_provider_peer_id,
                            signature: signature2,
                            sender
                        })
                        .await?;

                    receiver.await.map_err(|e| anyhow!(e.to_string()))
                }.boxed()
            });

        if let Ok(cfrags) = futures::future::try_join_all(requests).await {
            cfrags
        } else {
            vec![]
        }
    }

    fn parse_cfrags(
        &self,
        cfrags_raw: Vec<Result<Vec<u8>, SendError>>,
        capsule: umbral_pre::Capsule,
    ) -> Result<(Vec<VerifiedCapsuleFrag>, Vec<ReverieCapsulefrag>, u32)> {

        let mut verified_cfrags: Vec<VerifiedCapsuleFrag> = Vec::new();
        let mut new_vessel_cfrags: Vec<ReverieCapsulefrag> = Vec::new();
        let mut total_frags_received = 0;

        for cfrag_result in cfrags_raw.into_iter() {

            // Deserialize capsule fragments
            let reverie_cfrag: ReverieCapsulefrag = serde_json::from_slice(&cfrag_result?)?;
            let cfrag: umbral_pre::CapsuleFrag = serde_json::from_slice(&reverie_cfrag.umbral_capsule_frag)?;
            let new_vessel_pubkey  = reverie_cfrag.target_pubkey;

            total_frags_received += 1;

            info!("Success! cfrag({}) from {}\ntotal frags: {}",
                reverie_cfrag.frag_num,
                get_node_name(&reverie_cfrag.kfrag_provider_peer_id),
                total_frags_received
            );

            // Target vessel must check that cfrags are valid.
            let verified_cfrag = cfrag.verify(
                &capsule,
                &reverie_cfrag.source_verifying_pubkey, // verifying pk
                &reverie_cfrag.source_pubkey, // source pubkey
                &new_vessel_pubkey // target pubkey
            ).map_err(|(e, _)| anyhow!(e.to_string()))?;

            new_vessel_cfrags.push(reverie_cfrag);
            verified_cfrags.push(verified_cfrag);
        }

        Ok((verified_cfrags, new_vessel_cfrags, total_frags_received))
    }

    fn decrypt_cfrags(
        &self,
        reverie_msg: ReverieMessage,
        verified_cfrags: Vec<VerifiedCapsuleFrag>,
        new_vessel_cfrags: Vec<ReverieCapsulefrag>,
        total_frags_received: u32
    ) -> Result<AgentSecretsJson, Error> {

        // get next vessel
        let new_vessel_pubkey = match new_vessel_cfrags.iter().next() {
            Some(vessel) => vessel,
            None => return Err(anyhow!("No CapsuleFragments found"))
        };

        let threshold = new_vessel_pubkey.threshold as usize;
        info!("Received {}/{} required CapsuleFrags", total_frags_received, threshold);

        let capsule = serde_json::from_slice::<umbral_pre::Capsule>(&reverie_msg.reverie.umbral_capsule)?;

        // Bob (next target vessel) uses his umbral_key to open the capsule by using at
        // least `threshold` cfrags, then decrypts the re-encrypted ciphertext.
        match self.umbral_key.decrypt_reencrypted(
            &new_vessel_pubkey.source_pubkey, // delegator_pubkey
            &capsule, // capsule,
            verified_cfrags, // verified capsule fragments
            reverie_msg.reverie.umbral_ciphertext
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
                    warn!("Not enough fragments. Need {threshold}, received {total_frags_received}");
                } else {
                    warn!("Not decryptable by user {} with: {}", self.node_id.node_name, self.umbral_key.public_key);
                    warn!("Target decryptor pubkey: {}", new_vessel_pubkey.target_pubkey);
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

