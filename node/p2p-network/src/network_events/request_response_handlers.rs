use libp2p::request_response;
use colored::Colorize;
use tracing::info;
use crate::SendError;
use crate::types::{
    NetworkEvent,
    FragmentRequestEnum,
    FragmentResponseEnum,
    CapsuleFragmentMessage,
    AgentVesselInfo,
    NodeVesselWithStatus,
    VesselStatus,
    ReverieKeyfrag,
    ReverieCapsulefrag,
    ReverieKeyfragMessage,
    ReverieMessage,
};

use color_eyre::{Result, eyre::anyhow};
use crate::{short_peer_id, get_node_name};
use super::NetworkEvents;

//// Request Response Protocol
impl<'a> NetworkEvents<'a> {

    pub(super) async fn handle_request_response(
        &mut self,
        reqresp_event: request_response::Event<FragmentRequestEnum, FragmentResponseEnum>,
    ) -> Result<()> {
        match reqresp_event {
            request_response::Event::Message { message, .. } => {

                match message {

                    //////////////////////////////////
                    // Inbound Requests
                    //////////////////////////////////
                    request_response::Message::Request {
                        request: fragment_request,
                        channel,
                        ..
                    } => {
                        match fragment_request {
                            FragmentRequestEnum::GetFragmentRequest(
                                reverie_id,
                                frag_num,
                                kfrag_provider_peer_id
                            ) => {

                                info!("{}",
                                    format!("{} Inbound RequestFragmentRequest for {reverie_id} frag({frag_num}) from {}",
                                    self.nname(),
                                    short_peer_id(&kfrag_provider_peer_id)
                                ).yellow());


                                match self.peer_manager.get_cfrags(&reverie_id) {
                                    None => {
                                        tracing::warn!("{} No cfrag found for {} frag_num: {}", self.nname(), reverie_id, frag_num);
                                    },
                                    Some(cfrag) => {
                                        let cfrag_bytes =
                                            serde_json::to_vec::<Option<ReverieCapsulefrag>>(&Some(cfrag.clone()))
                                                .map_err(|e| SendError(e.to_string()));

                                        self.swarm.behaviour_mut()
                                            .request_response
                                            .send_response(
                                                channel,
                                                FragmentResponseEnum::FragmentResponse(cfrag_bytes)
                                            )
                                            .expect("Connection to peer to be still open.");

                                    }
                                }
                            },

                            FragmentRequestEnum::SaveFragmentRequest(
                                ReverieKeyfragMessage {
                                    reverie_keyfrag,
                                    source_peer_id,
                                    target_peer_id,
                                    ..
                                },
                                agent_name_nonce
                            ) => {

                                info!("\n\tReceived message: \n\t{:?}", reverie_keyfrag);
                                info!("\n\t{}\n\tFrom Peer: {} {}",
                                    format!("MessageType: SaveFragmentRequest").green(),
                                    get_node_name(&source_peer_id).green(),
                                    short_peer_id(&source_peer_id).yellow()
                                );

                                // 1) When a node receives a Kfrag, verify the Kfrag
                                let keyfrag: umbral_pre::KeyFrag = serde_json::from_slice(&reverie_keyfrag.umbral_keyfrag).expect("serde err");
                                let verified_kfrag = keyfrag.verify(
                                    &reverie_keyfrag.verifying_pk,
                                    Some(&reverie_keyfrag.alice_pk),
                                    Some(&reverie_keyfrag.bob_pk)
                                ).map_err(SendError::from).expect("keyfrag verification failed");

                                let capsule = serde_json::from_slice(&reverie_keyfrag.umbral_capsule).expect("serde err");
                                let cfrag = umbral_pre::reencrypt(&capsule, verified_kfrag).unverify();

                                // 2) Node stores Capsulefrags locally
                                self.peer_manager.insert_cfrags(
                                    &reverie_keyfrag.id,
                                    ReverieCapsulefrag {
                                        id: reverie_keyfrag.id.clone(),
                                        reverie_type: reverie_keyfrag.reverie_type,
                                        frag_num: reverie_keyfrag.frag_num,
                                        threshold: reverie_keyfrag.threshold,
                                        umbral_capsule_frag: serde_json::to_vec(&cfrag).expect("serde err"),
                                        alice_pk: reverie_keyfrag.alice_pk, // vessel
                                        bob_pk: reverie_keyfrag.bob_pk, // next vessel
                                        verifying_pk: reverie_keyfrag.verifying_pk,
                                        kfrag_provider_peer_id: self.node_id.peer_id,
                                    }
                                );

                                if let Some(agent_name_nonce) = agent_name_nonce {
                                    self.peer_manager.insert_reverie_metadata(
                                        &reverie_keyfrag.id,
                                        AgentVesselInfo {
                                            agent_name_nonce: agent_name_nonce,
                                            total_frags: reverie_keyfrag.total_frags,
                                            threshold: reverie_keyfrag.threshold,
                                            current_vessel_peer_id: source_peer_id,
                                            next_vessel_peer_id: target_peer_id,
                                        }
                                    );
                                }

                                // 3) Notify target vessel that this node is a KfragProvider for this ReverieId
                                let request_id = self.swarm.behaviour_mut()
                                    .request_response
                                    .send_request(
                                        &target_peer_id,
                                        FragmentRequestEnum::ProvidingFragmentRequest(
                                            reverie_keyfrag.id,
                                            reverie_keyfrag.frag_num,
                                            self.node_id.peer_id // kfrag_provider_peer_id
                                        )
                                    );

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

                                // 2). Respond to kfrag provider node and acknowledge receipt of Kfrag
                                self.swarm.behaviour_mut()
                                    .request_response
                                    .send_response(
                                        channel,
                                        FragmentResponseEnum::KfragProviderAck
                                    )
                                    .expect("Connection to peer to be still open.");

                            },

                            FragmentRequestEnum::SaveCiphertextRequest(
                                ReverieMessage {
                                    reverie,
                                    source_peer_id,
                                    target_peer_id,
                                },
                                agent_name_nonce
                            ) => {

                                info!("\n\tReceived message: \n\t{:?}", reverie);
                                info!("\n\t{}\n\tFrom Peer: {} {}",
                                    format!("MessageType: SaveCiphertextRequest").green(),
                                    get_node_name(&source_peer_id).yellow(),
                                    short_peer_id(&source_peer_id).yellow()
                                );

                                // 1) Save Agent metadata if need be
                                if let Some(agent_name_nonce) = agent_name_nonce {

                                    let agent_metadata = AgentVesselInfo {
                                        agent_name_nonce: agent_name_nonce,
                                        total_frags: reverie.total_frags,
                                        threshold: reverie.threshold,
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
                                        NodeVesselWithStatus {
                                            peer_id: self.node_id.peer_id,
                                            umbral_public_key: self.node_id.umbral_key.public_key,
                                            agent_vessel_info: Some(agent_metadata),
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
                                    },
                                );

                                // 3) Put reverie holder's PeerId on Kademlia
                                self.put_reverie_holder_kademlia(reverie.id, target_peer_id)
                                    .map_err(|e| anyhow!("Failed to put: {}", e))?;

                                // 3). Respond to broadcaster node and acknowledge receipt of Reverie/Ciphertext
                                self.swarm.behaviour_mut()
                                    .request_response
                                    .send_response(
                                        channel,
                                        FragmentResponseEnum::ReverieProviderAck
                                    ).map_err(|e| anyhow!("Failed to send: {:?}", e))?;
                            }
                        }
                    }

                    //////////////////////////////////
                    // Inbound Responses
                    //////////////////////////////////
                    request_response::Message::Response {
                        request_id,
                        response,
                    } => {
                        match response {
                            FragmentResponseEnum::FragmentResponse(cfrag_bytes) => {
                                info!("{}",
                                    format!("Responding with FragmentResponse for request_id: {request_id}").green()
                                );
                                // get sender channel associated with the request-response id
                                let sender = self.pending.request_fragments
                                    .remove(&request_id)
                                    .expect("request_response: Request pending.");

                                // send fragment to it
                                let _ = sender.send(cfrag_bytes);
                            }
                            FragmentResponseEnum::KfragProviderAck => {
                                info!("{} {}", self.nname(), format!("Acknowledged fragment provider\n").green());
                            }
                            FragmentResponseEnum::ReverieProviderAck => {
                                info!("{} {}", self.nname(), format!("Acknowledged ciphertext provider\n").green());
                            }
                        }
                    }
                }
            },
            request_response::Event::ResponseSent { .. } => {}
            request_response::Event::InboundFailure { .. } => {}
            request_response::Event::OutboundFailure {
                request_id,
                error,
                peer,
                ..
            } => {
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

