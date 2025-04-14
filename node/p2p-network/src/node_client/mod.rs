mod commands;
mod network_events_listener;
mod memories;
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
use serde::{Serialize, Deserialize, de::DeserializeOwned};

use crate::{get_node_name, short_peer_id, TryPeerId};
use crate::network_events::NodeIdentity;
use crate::types::{
    ReverieNameWithNonce,
    NetworkEvent,
    NodeKeysWithVesselStatus,
    RespawnId,
    Reverie,
    ReverieCapsulefrag,
    ReverieId,
    ReverieKeyfrag,
    ReverieKeyfragMessage,
    ReverieMessage,
    ReverieType,
    VesselStatus,
    SignatureType,
    VerifyingKey,
};
use crate::SendError;
use crate::behaviour::heartbeat_behaviour::TeePayloadOutEvent;
use runtime::reencrypt::{UmbralKey, VerifiedCapsuleFrag};
use runtime::llm::AgentSecretsJson;



#[derive(Clone)]
pub struct NodeClient<'a> {
    pub node_id: NodeIdentity<'a>,
    pub command_sender: mpsc::Sender<NodeCommand>,
    // container app state
    container_manager: Arc<tokio::sync::RwLock<ContainerManager>>,
    // hb subscriptions for rpc clients
    pub heartbeat_receiver: async_channel::Receiver<TeePayloadOutEvent>,
    // keep private in TEE
    // agent_secrets_json: Option<AgentSecretsJson>,
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
            command_sender: command_sender,
            // manages container reboot
            container_manager: container_manager,
            // hb subscriptions for rpc clients
            heartbeat_receiver: heartbeat_receiver,
            // keep private in TEE
            umbral_key: umbral_key,
        }
    }

    pub fn create_reverie<T: Serialize + DeserializeOwned>(
        &mut self,
        secrets: T,
        reverie_type: ReverieType,
        threshold: usize,
        total_frags: usize,
        target_public_key: umbral_pre::PublicKey,
        verifying_public_key: VerifyingKey,
    ) -> Result<Reverie> {

        let (
            capsule,
            ciphertext
        ) = self.umbral_key.encrypt_bytes(&serde_json::to_vec(&secrets)?)?;

        // Check secrets are decryptable and parsable
        println!("{}{}", "Encrypted AgentSecretsJson:\n",
            format!("{}", hex::encode(ciphertext.clone())).black()
        );
        let agent_secrets_json: Option<AgentSecretsJson> = serde_json::from_slice(
            &self.umbral_key.decrypt_original(&capsule, &ciphertext)?
        ).ok();

        let reverie = Reverie::new(
            "secrets description".to_string(),
            reverie_type,
            threshold,
            total_frags,
            target_public_key,
            verifying_public_key,
            capsule,
            ciphertext
        );

        Ok(reverie)
    }

    pub fn create_reverie_keyfrags(
        &self,
        reverie: &Reverie,
    ) -> Result<Vec<ReverieKeyfrag>> {
        // Alice generates reencryption key fragments for MPC nodes
        info!("Generating {}-of-{} keyfrags for reverie: {}", reverie.threshold, reverie.total_frags, reverie.id);

        let sign_target_key = match reverie.verifying_public_key {
            VerifyingKey::Umbral(..) => true, // verify kfrag belongs to a given target pubkey
            VerifyingKey::Ethereum(..) => false,
            VerifyingKey::Ed25519(..) => false,
        };

        let kfrags = self.umbral_key.generate_pre_keyfrags(
            &reverie.target_public_key,
            reverie.threshold,
            reverie.total_frags,
            true, // verify kfrag corresponds to a given delegating pubkey
            sign_target_key, // verify kfrag belongs to a given target pubkey
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
                target_pubkey: reverie.target_public_key,
                source_verifying_pubkey: self.umbral_key.verifying_public_key,
                target_verifying_pubkey: reverie.verifying_public_key.clone(),
            }
        }).collect::<Vec<ReverieKeyfrag>>();

        Ok(kfrags)
    }

    pub async fn get_prospect_vessels(
        &self,
        shuffle: bool
    ) -> Result<(NodeKeysWithVesselStatus, Vec<NodeKeysWithVesselStatus>)> {

        let peer_nodes = self.get_node_vessels(shuffle).await
            .into_iter()
            .filter(|v| v.vessel_status == VesselStatus::EmptyVessel)
            .collect::<Vec<NodeKeysWithVesselStatus>>();

        let (
            target_vessel,
            target_kfrag_providers
        ) = peer_nodes
            .split_first()
            .ok_or(anyhow!("No Peers found."))?;

        if target_kfrag_providers.contains(&target_vessel) {
            return Err(anyhow!("Target vessel cannot also be a kfrag provider"));
        }

        Ok((target_vessel.clone(), target_kfrag_providers.to_vec()))
    }

    pub async fn broadcast_reverie_keyfrags(
        &mut self,
        reverie: &Reverie,
        target_vessel_peer_id: PeerId,
        target_kfrag_providers: Vec<NodeKeysWithVesselStatus>
    ) -> Result<()> {

        let umbral_ciphertext = reverie.umbral_ciphertext.clone();

        // Split into fragments
        let kfrags = self.create_reverie_keyfrags(&reverie)?;

        if target_kfrag_providers.len() < reverie.total_frags {
            return Err(anyhow!("Not connected to enough peers, need: {}, got: {}", reverie.total_frags + 1, target_kfrag_providers.len() + 1));
        }
        info!("Kfrag providers: {}", target_kfrag_providers.len());
        info!("Total frags: {}", reverie.total_frags);

        // Send Kfrags to peer nodes
        for (i, reverie_keyfrag) in kfrags.into_iter().enumerate() {
            let keyfrag_provider = target_kfrag_providers[i].peer_id;
            self.command_sender.send(
                NodeCommand::SendReverieKeyfrag {
                    keyfrag_provider: keyfrag_provider,
                    reverie_keyfrag_msg: ReverieKeyfragMessage {
                        reverie_keyfrag: reverie_keyfrag,
                        source_peer_id: self.node_id.peer_id,
                        target_peer_id: target_vessel_peer_id,
                    },
                }
            ).await?;
        }

        // Add Memory trading:
        // make it easy for a user ALICE (not running a node) to
        // - submit a memory + PRE it for BOB (not running a node)
        // - BOB to decrypt and run the memory as context for a TEE Agent

        // Expand memories to MCP plugins
        // - Alice can now upload MCP plugins with API keys and rent access to them to BOB
        // - Alice can even delegate access to entire github repos to execute in TEEs to BOB
        // - Problem is marketing the MCP plugins: how does BOB know what the plugin does without seeing the code?
        // - Use an AI description of the plugin? document endpoints?

        match reverie.reverie_type {
            ReverieType::SovereignAgent(..) => {
                // Send Ciphertext to target vessel if ReverieType::SovereignAgent
                // These agents live in an isolated TEE instance
                self.command_sender.send(
                    NodeCommand::SendReverieToSpecificPeer {
                        ciphertext_holder: target_vessel_peer_id, // Ciphertext Holder
                        reverie_msg: ReverieMessage {
                            reverie: reverie.clone(),
                            source_peer_id: self.node_id.peer_id,
                            target_peer_id: target_vessel_peer_id,
                        },
                    }
                ).await?;
            }
            _ => {
                // Save Reverie on DHT for other ReverieTypes (Agent, Memory, Retrieval, Tools)
                self.command_sender.send(
                    NodeCommand::SaveReverieOnNetwork {
                        reverie_msg: ReverieMessage {
                            reverie: reverie.clone(),
                            source_peer_id: self.node_id.peer_id,
                            target_peer_id: target_vessel_peer_id,
                        },
                    }
                ).await?;

                // Still need to send to the target vessel for now
                // TODO: refactor to allow any peer to request the reverie and kfrags
                // instread of needing a specific peer/node for non-SovereignAgents
                self.command_sender.send(
                    NodeCommand::SendReverieToSpecificPeer {
                        ciphertext_holder: target_vessel_peer_id, // Ciphertext Holder
                        reverie_msg: ReverieMessage {
                            reverie: reverie.clone(),
                            source_peer_id: self.node_id.peer_id,
                            target_peer_id: target_vessel_peer_id,
                        },
                    }
                ).await?;
            }
        }

        Ok(())
    }

    pub async fn get_node_vessels(&self, shuffle: bool) -> Vec<NodeKeysWithVesselStatus> {
        let (sender, mut receiver) = mpsc::channel(100);
        self.command_sender
            .send(NodeCommand::GetNodeVesselStatusesFromKademlia { sender })
            .await
            .expect("Command receiver not to be dropped.");

        let mut pks: Vec<NodeKeysWithVesselStatus> = vec![];
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

    pub async fn get_reverie(&self, reverie_id: ReverieId, reverie_type: ReverieType) -> Result<ReverieMessage> {
        let (sender, receiver) = oneshot::channel();

        self.command_sender
            .send(NodeCommand::GetReverie {
                reverie_id: reverie_id,
                reverie_type: reverie_type,
                sender: sender
            })
            .await?;

        receiver.await.map_err(SendError::from)?
    }

    pub async fn request_cfrags(
        &mut self,
        reverie_id: ReverieId,
        external_signature: Option<SignatureType>
    ) -> Vec<Result<Vec<u8>, SendError>> {

        let providers = self.get_kfrag_providers(reverie_id.clone()).await;

        info!("Finding kfrag providers for: {}", reverie_id);
        info!("Kfrag providers: {:?}\n", providers);

        // Get either the provided signature or generate one using umbral_key
        let signature = match external_signature {
            Some(sig) => sig,
            None => {
                // target vessel creates signature by signing the digest hash of reverie_id
                let digest = Keccak256::digest(reverie_id.clone().as_bytes());
                // Sign with our umbral signer key (corresponds to the verifying key)
                let umbral_signature = self.umbral_key.sign(&digest);
                SignatureType::Umbral(
                    serde_json::to_vec(&umbral_signature)
                        .expect("Failed to serialize umbral signature")
                )
            }
        };

        // Rest of the function using signature
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
                            kfrag_provider_peer_id: kfrag_provider_peer_id.clone(),
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
    ) -> Result<(Vec<VerifiedCapsuleFrag>, umbral_pre::PublicKey, usize)> {

        let mut verified_cfrags: Vec<VerifiedCapsuleFrag> = Vec::new();
        let mut reverie_cfrags: Vec<ReverieCapsulefrag> = Vec::new();

        let mut required_threshold = 0;
        let mut total_frags_received = 0;

        for cfrag_result in cfrags_raw.into_iter() {

            // Deserialize capsule fragments
            let reverie_cfrag: ReverieCapsulefrag = serde_json::from_slice(&cfrag_result?)?;
            let cfrag = reverie_cfrag.encode_capsule_frag()?;
            let new_vessel_pubkey  = reverie_cfrag.target_pubkey;

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
                &reverie_cfrag.target_pubkey // target pubkey
            ).map_err(|(e, _)| anyhow!(e.to_string()))?;

            total_frags_received += 1;
            required_threshold = reverie_cfrag.threshold;
            verified_cfrags.push(verified_cfrag);
            reverie_cfrags.push(reverie_cfrag);
        }

        info!("Received {}/{} required CapsuleFrags", total_frags_received, required_threshold);

        if total_frags_received < required_threshold {
            warn!("Not enough fragments. Need {required_threshold}, received {total_frags_received}");
            return Err(anyhow!("Insufficient cfrags fragments received"))
        }

        // delegator pubkey
        let first_cfrag = match reverie_cfrags.iter().next() {
            Some(cfrag) => cfrag.clone(),
            None => return Err(anyhow!("No cfrags received"))
        };

        // Validate that all capsule fragments are from the same Reverie
        let valid_cfrags = reverie_cfrags.iter().all(|cfrag| {
            cfrag.source_pubkey == first_cfrag.source_pubkey &&
            cfrag.target_pubkey == first_cfrag.target_pubkey &&
            cfrag.source_verifying_pubkey == first_cfrag.source_verifying_pubkey &&
            cfrag.target_verifying_pubkey == first_cfrag.target_verifying_pubkey
        });

        Ok((verified_cfrags, first_cfrag.source_pubkey, total_frags_received))
    }

    fn decrypt_cfrags<T: Serialize + DeserializeOwned>(
        &self,
        capsule: umbral_pre::Capsule,
        ciphertext: Box<[u8]>,
        source_pubkey: umbral_pre::PublicKey,
        verified_cfrags: Vec<VerifiedCapsuleFrag>,
    ) -> Result<T, Error> {

        // Bob (next target vessel) uses his umbral_key to open the capsule by using at
        // least threshold cfrags, then decrypts the re-encrypted ciphertext.
        match self.umbral_key.decrypt_reencrypted(
            &source_pubkey, // delegator pubkey
            &capsule, // capsule,
            verified_cfrags, // verified capsule fragments
            ciphertext // ciphertext
        ) {
            Ok(plaintext_bob) => {
                let decrypted_data: serde_json::Value = serde_json::from_slice(&plaintext_bob)?;
                let secrets_str = serde_json::to_string_pretty(&decrypted_data)?;
                println!("Decrypted (re-encrypted) secrets:\n{}", format!("{}", secrets_str).bright_black());
                let secrets_json = serde_json::from_slice(&plaintext_bob)?;
                Ok(secrets_json)
            },
            Err(e) => {
                error!("{}", e);
                warn!("Not decryptable by user {} with: {}", self.node_id.node_name, self.umbral_key.public_key);
                warn!("Target decryptor pubkey: {}", source_pubkey);
                Err(anyhow!(e.to_string()))
            }
        }
    }

    fn nname(&self) -> String {
        format!("{}{}", self.node_id.node_name.yellow(), ">".blue())
    }

    pub async fn get_reverie_id_by_name(&self, reverie_name_nonce: &ReverieNameWithNonce) -> Option<ReverieId> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.command_sender
            .send(NodeCommand::GetReverieIdByName {
                reverie_name_nonce: reverie_name_nonce.clone(),
                sender: sender,
            })
            .await.expect("Command receiver not to bee dropped");

        receiver.await.expect("get reverie receiver not to drop")
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

