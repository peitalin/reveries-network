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
    KeyFragmentMessage2,
};
use crate::short_peer_id;
use super::NetworkEvents;


//// Request Response Protocol
impl<'a> NetworkEvents<'a> {
    pub(super) async fn handle_request_response(
        &mut self,
        reqresp_event: request_response::Event<FragmentRequestEnum, FragmentResponseEnum>,
    ) {
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
                                self.network_event_sender
                                    .send(NetworkEvent::InboundCapsuleFragRequest {
                                        reverie_id,
                                        frag_num,
                                        kfrag_provider_peer_id,
                                        channel,
                                    })
                                    .await
                                    .expect("Event receiver not to be dropped.");
                            },
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

                                // 2). Respond to broadcasting node and acknowledge receipt of Kfrag
                                self.swarm.behaviour_mut()
                                    .request_response
                                    .send_response(
                                        channel,
                                        FragmentResponseEnum::KfragProviderAck
                                    )
                                    .expect("Connection to peer to be still open.");

                            },
                            FragmentRequestEnum::SaveFragmentRequest(
                                KeyFragmentMessage2 {
                                    reverie_keyfrag,
                                    frag_num,
                                    total_frags,
                                    source_peer_id,
                                    target_peer_id,
                                    ..
                                },
                                agent_name_nonce
                            ) => {

                                // When a node receives a Kfrag and Capsule
                                let keyfrag: umbral_pre::KeyFrag = serde_json::from_slice(&reverie_keyfrag.umbral_keyfrag).expect("serde err");
                                let verified_kfrag = keyfrag.verify(
                                    &reverie_keyfrag.verifying_pk,
                                    Some(&reverie_keyfrag.alice_pk),
                                    Some(&reverie_keyfrag.bob_pk)
                                ).map_err(SendError::from).expect("keyfrag verification failed");

                                let capsule = serde_json::from_slice(&reverie_keyfrag.umbral_capsule).expect("serde err");
                                let cfrag = umbral_pre::reencrypt(&capsule, verified_kfrag).unverify();

                                // node stores cfrags locally
                                // nodes should also inform the vessel_node that they hold a fragment
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
                                    }
                                );

                                if let Some(agent_name_nonce) = agent_name_nonce {
                                    self.peer_manager.insert_reverie_metadata(
                                        &reverie_keyfrag.id,
                                        AgentVesselInfo {
                                            agent_name_nonce: agent_name_nonce,
                                            total_frags: total_frags,
                                            current_vessel_peer_id: source_peer_id,
                                            next_vessel_peer_id: target_peer_id,
                                        }
                                    );
                                }

                                // if let Some(peer_info) = self.peer_manager.peer_info.get(&k.vessel_peer_id) {
                                //     match &peer_info.agent_vessel {
                                //         None => {
                                //             let agent_vessel_info = AgentVesselInfo {
                                //                 agent_name_nonce: agent_name_nonce.clone(),
                                //                 total_frags,
                                //                 next_vessel_peer_id: k.next_vessel_peer_id,
                                //                 current_vessel_peer_id: k.vessel_peer_id,
                                //             };

                                //             self.peer_manager.set_peer_info_agent_vessel(&agent_vessel_info);

                                //             // Put signed vessel status
                                //             let status = NodeVesselWithStatus {
                                //                 peer_id: k.vessel_peer_id,
                                //                 umbral_public_key: k.alice_pk,
                                //                 agent_vessel_info: Some(agent_vessel_info),
                                //                 vessel_status: VesselStatus::ActiveVessel,
                                //             };
                                //             self.put_signed_vessel_status(status)?;
                                //         }
                                //         Some(existing_agent) => {
                                //             panic!("can't replace existing agent in node: {:?}", existing_agent);
                                //         }
                                //     }
                                // }

                                // 3) Notify target vessel that this node is a KfragProvider for this ReverieId
                                let request_id = self.swarm.behaviour_mut()
                                    .request_response
                                    .send_request(
                                        &target_peer_id,
                                        FragmentRequestEnum::ProvidingFragmentRequest(
                                            reverie_keyfrag.id,
                                            frag_num,
                                            self.node_id.peer_id // kfrag_provider_peer_id
                                        )
                                    );

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
                            FragmentResponseEnum::FragmentResponse(fragment_bytes) => {
                                // get sender channel associated with the request-response id
                                let sender = self.pending.request_fragments
                                    .remove(&request_id)
                                    .expect("request_response: Request pending.");
                                // send fragment to it
                                let _ = sender.send(fragment_bytes);
                            }
                            FragmentResponseEnum::KfragProviderAck => {
                                info!("{} {}", self.nname(), format!("vessel acknowledged fragment provider\n").green());
                                // let sender = self.pending.send_fragments
                                //     .remove(&request_id)
                                //     .expect("request_response: Request pending.");
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
                    None => tracing::warn!("RequestId {} not found for {}", request_id, peer),
                    Some(sender) => {
                        sender.send(Err(SendError(error.to_string()))).ok();
                    }
                }
            }
        }
    }
}

