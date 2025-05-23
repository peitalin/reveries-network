use colored::Colorize;
use color_eyre::{Result, eyre::anyhow};
use libp2p::request_response;
use libp2p::request_response::{Event, Message};
use tracing::{info, warn};
use sha3::{Digest, Keccak256};

use crate::SendError;
use crate::types::{
    NetworkEvent,
    FragmentRequestEnum,
    FragmentResponseEnum,
    AgentVesselInfo,
    NodeKeysWithVesselStatus,
    VesselStatus,
    ReverieKeyfrag,
    ReverieCapsulefrag,
    ReverieKeyfragMessage,
    ReverieMessage,
    ReverieType,
    AccessKey,
};
use crate::{short_peer_id, get_node_name, get_node_name2};
use crate::types::create_digest_hash;
use super::NetworkEvents;

type RequestResponseEvent = Event<FragmentRequestEnum, FragmentResponseEnum>;

//// Request Response Protocol
impl NetworkEvents {
    pub(super) async fn handle_request_response(&mut self, rr_event: RequestResponseEvent) -> Result<()> {
        match rr_event {

            //////////////////////////////////////////////
            //// 1) Message Received - Inbound Requests
            //////////////////////////////////////////////
            Event::Message {
                peer,
                connection_id,
                message: Message::Request {
                    request_id,
                    request: fragment_request,
                    channel,
                },
            } => {
                match fragment_request {
                    FragmentRequestEnum::GetFragmentRequest(
                        reverie_id,
                        access_key
                    ) => {

                        info!("{}", format!("{} Inbound RequestFragmentRequest {reverie_id}", self.nname()).yellow());
                        info!("{}", format!("Signature: {access_key}").yellow());

                        let cfrag = match self.peer_manager.get_cfrags(&reverie_id) {
                            Some(cfrag) => cfrag,
                            None => return Err(anyhow!("{} No cfrag found for {}", self.nname(), reverie_id)),
                        };

                        // TODO: refactor to a AccessCondition / Auth module for generalizing access control and conditions
                        // Verify signature is from intended recipient/target before sending capsule fragment
                        // TODO: add nonce and timestamp to digest
                        match &access_key {
                            AccessKey::NearContract(
                                contract_account_id,
                                spender_account_id,
                                amount
                            ) => {

                                println!("LLLLLL; reverie_id: {:?}", reverie_id);
                                println!("KKKKKK; contract_account_id: {:?}", contract_account_id);
                                println!("MMMMMM; spender_account_id: {:?}", spender_account_id);
                                println!("JJJJJJ; amount: {:?}", amount);

                                let can_spend = self.near_runtime.can_spend(
                                    contract_account_id,
                                    &reverie_id,
                                    spender_account_id,
                                    *amount
                                ).await?;

                                if can_spend {
                                    info!("{}", format!("Near Contract Access granted!").green());
                                } else {
                                    return Err(anyhow!("Near Contract Access denied!"));
                                }
                            }
                            AccessKey::EcdsaSignature(signature) => {
                                match access_key.verify_access(&cfrag.access_condition, reverie_id) {
                                    false => return Err(anyhow!("Invalid signature for fragment request for {reverie_id}".to_string())),
                                    true => info!("{}", format!("Signature verified!").green())
                                }
                            }
                            AccessKey::UmbralSignature(signature) => {
                                match access_key.verify_access(&cfrag.access_condition, reverie_id) {
                                    false => return Err(anyhow!("Invalid signature for fragment request for {reverie_id}".to_string())),
                                    true => info!("{}", format!("Signature verified!").green())
                                }
                            }
                            AccessKey::Ed25519Signature(signature) => {
                                return Err(anyhow!("Ed25519 signatures are not implemented yet"));
                            }
                            AccessKey::EthContract(
                                contract_address,
                                contract_method_name,
                                contract_args
                            ) => {
                                return Err(anyhow!("EthContract access keys are not implemented yet"));
                            }
                        };

                        let cfrag_bytes = serde_json::to_vec::<ReverieCapsulefrag>(&cfrag)
                            .map_err(|e| SendError(e.to_string()));

                        self.swarm.behaviour_mut()
                            .request_response
                            .send_response(
                                channel,
                                FragmentResponseEnum::GetFragmentResponse(cfrag_bytes)
                            )
                            .expect("Connection to peer to be still open.");
                    },

                    FragmentRequestEnum::SaveFragmentRequest(
                        ReverieKeyfragMessage {
                            reverie_keyfrag,
                            source_peer_id,
                            target_peer_id,
                            ..
                        },
                    ) => {

                        info!("\n\t{}\n\tfrom Peer: {} {}",
                            format!("Received MessageType: SaveFragmentRequest").green(),
                            get_node_name(&source_peer_id).green(),
                            short_peer_id(&source_peer_id).yellow()
                        );

                        // 1) When a node receives a Kfrag, verify the Kfrag
                        let keyfrag: umbral_pre::KeyFrag = serde_json::from_slice(&reverie_keyfrag.umbral_keyfrag).expect("serde err");
                        let verified_kfrag = keyfrag.verify(
                            &reverie_keyfrag.source_verifying_pubkey,
                            Some(&reverie_keyfrag.source_pubkey),
                            Some(&reverie_keyfrag.target_pubkey)
                        ).map_err(SendError::from).expect("keyfrag verification failed");

                        let capsule = serde_json::from_slice(&reverie_keyfrag.umbral_capsule).expect("serde err");
                        let cfrag = umbral_pre::reencrypt(&capsule, verified_kfrag).unverify();

                        // 2) Save Agent metadata if ReverieType is Agent
                        if let ReverieType::Agent(..) | ReverieType::SovereignAgent(..) = reverie_keyfrag.reverie_type {

                            let agent_metadata = AgentVesselInfo {
                                reverie_id: reverie_keyfrag.id.clone(),
                                reverie_type: reverie_keyfrag.reverie_type.clone(),
                                threshold: reverie_keyfrag.threshold,
                                total_frags: reverie_keyfrag.total_frags,
                                current_vessel_peer_id: source_peer_id,
                                next_vessel_peer_id: target_peer_id,
                            };

                            self.peer_manager.set_peer_info_agent_vessel(&agent_metadata);
                            self.peer_manager.insert_reverie_metadata(
                                &reverie_keyfrag.id,
                                agent_metadata
                            );
                        }

                        // 3) Node stores Capsulefrags locally
                        self.peer_manager.insert_cfrags(
                            &reverie_keyfrag.id,
                            ReverieCapsulefrag {
                                id: reverie_keyfrag.id.clone(),
                                reverie_type: reverie_keyfrag.reverie_type,
                                frag_num: reverie_keyfrag.frag_num,
                                threshold: reverie_keyfrag.threshold,
                                umbral_capsule_frag: serde_json::to_vec(&cfrag).expect("serde err"),
                                source_pubkey: reverie_keyfrag.source_pubkey, // source vessel
                                source_verifying_pubkey: reverie_keyfrag.source_verifying_pubkey, // source vessel verifying key
                                target_pubkey: reverie_keyfrag.target_pubkey, // target vessel
                                target_verifying_pubkey: reverie_keyfrag.target_verifying_pubkey, // target vessel verifying key
                                access_condition: reverie_keyfrag.access_condition, // access condition to be checked against to request cfrags
                                kfrag_provider_peer_id: self.node_id.peer_id,
                            }
                        );

                        // 3) Notify target vessel that this node is a KfragProvider for this ReverieId
                        let request_id = self.swarm.behaviour_mut().request_response
                            .send_request(
                                &target_peer_id,
                                FragmentRequestEnum::ProvidingFragmentRequest(
                                    reverie_keyfrag.id,
                                    reverie_keyfrag.frag_num,
                                    self.node_id.peer_id // kfrag_provider_peer_id
                                )
                            );

                        // 4). Respond to broadcaster node, acknowledging receipt of Kfrag
                        self.swarm.behaviour_mut().request_response
                            .send_response(
                                channel,
                                FragmentResponseEnum::SaveFragmentResponse
                            ).expect("Connection to peer to be still open.");

                    },

                    // Providers let Reverie holder know they are a Kfrag provider
                    FragmentRequestEnum::ProvidingFragmentRequest(
                        reverie_id,
                        frag_num,
                        kfrag_provider_peer_id
                    ) => {

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

                        // 2). Respond to Kfrag Provider and peer as Provider
                        self.swarm.behaviour_mut().request_response
                            .send_response(
                                channel,
                                FragmentResponseEnum::ProvidingFragmentResponse
                            )
                            .expect("Connection to peer to be still open.");

                    },

                    FragmentRequestEnum::SaveCiphertextRequest(
                        ReverieMessage {
                            reverie,
                            source_peer_id,
                            target_peer_id,
                            keyfrag_providers,
                        },
                    ) => {

                        info!("\n\t{}\n\tfrom peer: {} {}",
                            format!("Received MessageType: SaveCiphertextRequest").green(),
                            get_node_name(&source_peer_id).yellow(),
                            short_peer_id(&source_peer_id).yellow()
                        );

                        // 1) Save Agent metadata if need be
                        if let ReverieType::Agent(..) | ReverieType::SovereignAgent(..) = reverie.reverie_type {

                            let agent_metadata = AgentVesselInfo {
                                reverie_id: reverie.id.clone(),
                                reverie_type: reverie.reverie_type.clone(),
                                threshold: reverie.threshold,
                                total_frags: reverie.total_frags,
                                current_vessel_peer_id: source_peer_id,
                                next_vessel_peer_id: target_peer_id,
                            };

                            self.peer_manager.set_peer_info_agent_vessel(&agent_metadata);
                            self.peer_manager.insert_reverie_metadata(
                                &reverie.id,
                                agent_metadata.clone()
                            );

                            // Put signed vessel status on Kademlia
                            self.put_signed_vessel_status_kademlia(
                                NodeKeysWithVesselStatus {
                                    peer_id: self.node_id.peer_id,
                                    umbral_public_key: self.node_id.umbral_key.public_key,
                                    umbral_verifying_public_key: self.node_id.umbral_key.verifying_public_key,
                                    vessel_status: VesselStatus::ActiveVessel,
                                }
                            ).expect("Failed to put signed vessel status on Kademlia");
                        }

                        // 2) Save Reverie locally on this node
                        self.peer_manager.insert_reverie(
                            &reverie.id,
                            ReverieMessage {
                                reverie: reverie.clone(),
                                source_peer_id,
                                target_peer_id,
                                keyfrag_providers,
                            },
                        );

                        // 3) Put reverie holder's PeerId on Kademlia
                        self.put_reverie_holder_kademlia(reverie.id, target_peer_id)
                            .map_err(|e| anyhow!("Failed to put: {}", e))?;

                        // 4). Respond to broadcaster node and acknowledge receipt of Reverie/Ciphertext
                        self.swarm.behaviour_mut().request_response
                            .send_response(
                                channel,
                                FragmentResponseEnum::SaveCiphertextResponse
                            ).map_err(|e| anyhow!("Failed to send: {:?}", e))?;
                    }

                    FragmentRequestEnum::MarkRespawnCompleteRequest {
                        prev_reverie_id,
                        prev_peer_id,
                        prev_agent_name
                    } => {
                        info!("Inbound MarkRespawnCompleteRequest");
                        self.mark_pending_respawn_complete(prev_peer_id, prev_agent_name);

                        self.swarm.behaviour_mut().request_response
                            .send_response(
                                channel,
                                FragmentResponseEnum::MarkRespawnCompleteResponse
                            ).map_err(|e| anyhow!("Failed to send: {:?}", e))?;
                    }
                }
            }

            //// 2) Response Sent
            Event::ResponseSent { peer, request_id, .. } => {
                info!("{}", format!("ResponseSent to {} for request_id: {}", short_peer_id(peer), request_id).green());
            }

            //////////////////////////////////////////////
            //// 3) Response Received - Inbound Responses
            //////////////////////////////////////////////
            Event::Message {
                peer,
                connection_id,
                message: Message::Response {
                    request_id,
                    response,
                },
            } => {
                let peer_name = get_node_name2(&peer);
                match response {
                    FragmentResponseEnum::GetFragmentResponse(cfrag_bytes) => {
                        info!("{}", format!("RequestId({request_id}) Received GetFragmentResponse from {peer_name}").green());
                        // get sender channel associated with the request-response id
                        let sender = self.pending.request_fragments
                            .remove(&request_id)
                            .expect("request_response: Request pending.");

                        // send fragment to it
                        sender.send(cfrag_bytes).ok();
                    }
                    FragmentResponseEnum::ProvidingFragmentResponse => {
                        info!("{}", format!("RequestId({request_id}) Received ProvidingFragmentResponse from {peer_name}").green());
                    }
                    FragmentResponseEnum::SaveFragmentResponse => {
                        info!("{}", format!("RequestId({request_id}) Received SaveFragmentResponse from {peer_name}").green());
                    }
                    FragmentResponseEnum::SaveCiphertextResponse => {
                        info!("{}", format!("RequestId({request_id}) Received SaveCiphertextResponse from {peer_name}").green());
                    }
                    FragmentResponseEnum::MarkRespawnCompleteResponse => {
                        info!("{}", format!("RequestId({request_id}) Received MarkRespawnCompleteResponse from {peer_name}").green());
                    }
                }
            },
            Event::InboundFailure { .. } => {}
            Event::OutboundFailure { request_id, error, peer, ..  } => {
                match self.pending.request_fragments.remove(&request_id) {
                    None => tracing::warn!("RequestId({}) not found for {}", request_id, peer),
                    Some(sender) => {
                        sender.send(Err(SendError(error.to_string()))).ok();
                    }
                }
            }
        }

        Ok(())
    }
}

