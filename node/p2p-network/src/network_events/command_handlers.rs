use color_eyre::{Result, eyre::anyhow};
use colored::Colorize;
use libp2p::{kad, PeerId};
use tracing::{info, error};

use crate::node_client::NodeCommand;
use crate::types::{
    AgentVesselInfo,
    FragmentRequestEnum,
    FragmentResponseEnum,
    NodeVesselWithStatus,
    VesselStatus,
    VesselPeerId,
    ReverieKeyfragMessage,
    ReverieMessage,
    ReverieType,
    ReverieKeyfrag,
    ReverieCapsulefrag,
    Reverie,
    ReverieIdToAgentName,
    ReverieIdToPeerId,
};
use crate::{get_node_name, short_peer_id};
use crate::SendError;
use super::NetworkEvents;


impl<'a> NetworkEvents<'a> {
    pub(crate) async fn handle_command(&mut self, command: NodeCommand) {
        match command {

            NodeCommand::SendReverieKeyfrag {
                keyfrag_provider, // Key Fragment Provider
                reverie_keyfrag_msg: ReverieKeyfragMessage {
                    reverie_keyfrag,
                    source_peer_id,
                    target_peer_id,
                },
                agent_name_nonce,
            } => {
                let _request_id = self.swarm.behaviour_mut()
                    .request_response
                    .send_request(
                        &keyfrag_provider,
                        FragmentRequestEnum::SaveFragmentRequest(
                            ReverieKeyfragMessage {
                                reverie_keyfrag,
                                source_peer_id,
                                target_peer_id,
                            },
                            agent_name_nonce
                        )
                    );
            }
            NodeCommand::SendReverie {
                ciphertext_holder, // Ciphertext Holder
                reverie_msg: ReverieMessage {
                    reverie,
                    source_peer_id,
                    target_peer_id,
                },
                agent_name_nonce,
            } => {

                // 1. Save agent metadata
                if let Some(agent_name_nonce) = &agent_name_nonce {

                    // Set broadcaster's peer info
                    self.peer_manager.set_peer_info_agent_vessel(
                        &AgentVesselInfo {
                            agent_name_nonce: agent_name_nonce.clone(),
                            reverie_id: reverie.id.clone(),
                            total_frags: reverie.total_frags,
                            threshold: reverie.threshold,
                            current_vessel_peer_id: source_peer_id,
                            next_vessel_peer_id: target_peer_id,
                        }
                    );

                    // Put agent_name_nonce => reverie_id on DHT
                    let reverie_id_bytes = serde_json::to_vec(&reverie.id)
                        .expect("serde_json::to_vec(reverie_id)");

                    self.swarm.behaviour_mut().kademlia.put_record(
                        kad::Record {
                            key: agent_name_nonce.to_reverie_id().to_kad_key(),
                            value: reverie_id_bytes,
                            publisher: Some(self.node_id.peer_id),
                            expires: None,
                        },
                        kad::Quorum::Majority
                    ).expect("put_record err");
                }

                // Dispatch Reverie (ciphertext) to target vessel
                let _request_id = self.swarm.behaviour_mut()
                    .request_response
                    .send_request(
                        &ciphertext_holder,
                        FragmentRequestEnum::SaveCiphertextRequest(
                            ReverieMessage {
                                reverie,
                                source_peer_id,
                                target_peer_id,
                            },
                            agent_name_nonce.clone()
                        )
                    );
            }
            NodeCommand::GetReverie {
                reverie_id,
                sender,
            } => {
                info!("{}", format!("GetReverie for: {}", reverie_id).green());
                let reverie = match self.peer_manager.get_reverie(&reverie_id) {
                    Some(reverie) => Ok(reverie.clone()),
                    None => Err(anyhow!("Reverie not found")),
                };
                sender.send(reverie).ok();
            }
            NodeCommand::GetNodeVesselStatusesFromKademlia { sender } => {

                let peers = self.swarm.connected_peers()
                    .cloned()
                    .collect::<Vec<PeerId>>();

                for peer_id in peers {
                    // add prefix as kademlia key
                    let vessel_kademlia_key = VesselPeerId::from(peer_id);
                    self.swarm.behaviour_mut()
                        .kademlia
                        .get_record(kad::RecordKey::new(&vessel_kademlia_key.to_string()));

                    self.pending.get_node_vessels.insert(vessel_kademlia_key, sender.clone());
                };
            }
            NodeCommand::GetReverieIdFromAgentName {
                agent_name_nonce,
                sender,
            } => {
                // add prefix as kademlia key
                let agent_reverie_kadkey = ReverieIdToAgentName::from(agent_name_nonce.clone());

                self.swarm.behaviour_mut()
                    .kademlia
                    .get_record(kad::RecordKey::new(&agent_reverie_kadkey.to_string()));

                self.pending.get_reverie_agent_name.insert(agent_reverie_kadkey, sender);
            }
            NodeCommand::GetKfragProviders { reverie_id, sender } => {
                match self.peer_manager.kfrag_providers.get(&reverie_id) {
                    None => {
                        error!("missing kfrag_providers: {:?}", self.peer_manager.kfrag_providers);
                        sender.send(std::collections::HashSet::new()).ok();
                    }
                    Some(peers) => {
                        // unsorted
                        sender.send(peers.clone()).ok();
                    }
                }
            }
            NodeCommand::RequestCapsuleFragment {
                reverie_id,
                kfrag_provider_peer_id,
                signature,
                sender,
            } => {

                info!("{}",
                    format!("RequestCapsuleFragment for {} from {} {}",
                        reverie_id,
                        get_node_name(&kfrag_provider_peer_id),
                        short_peer_id(&kfrag_provider_peer_id)
                    ).yellow()
                );

                let request_id = self.swarm.behaviour_mut()
                    .request_response
                    .send_request(
                        &kfrag_provider_peer_id,
                        FragmentRequestEnum::GetFragmentRequest(
                            reverie_id,
                            signature
                        )
                    );

                self.pending.request_fragments.insert(request_id, sender);
            }
            NodeCommand::MarkPendingRespawnComplete {
                prev_reverie_id,
                prev_peer_id, // failed vessel's peer_id
                prev_agent_name_nonce,
            } => {

                // 1) Mark respawn complete locally
                self.mark_pending_respawn_complete(prev_peer_id, prev_agent_name_nonce.clone());

                // 2) Get previous kfrag providers of the previous vessel
                match self.peer_manager.kfrag_providers.get(&prev_reverie_id) {
                    None => {
                        tracing::warn!("No previous kfrag providers found for: {}", prev_reverie_id);
                    },
                    Some(prev_kfrag_providers) => {
                        // 3) Broadcast to previous Kfrag Providers to do the same
                        for peer_id in prev_kfrag_providers {
                            self.swarm.behaviour_mut()
                                .request_response
                                .send_request(
                                    &peer_id,
                                    FragmentRequestEnum::MarkRespawnCompleteRequest {
                                        prev_reverie_id: prev_reverie_id.clone(),
                                        prev_peer_id: prev_peer_id.clone(),
                                        prev_agent_name: prev_agent_name_nonce.clone(),
                                    }
                                );
                        }
                    }
                }

            }
            NodeCommand::StartListening { addr, sender } => {
                let _ = match self.swarm.listen_on(addr) {
                    Ok(_) => sender.send(Ok(())),
                    Err(e) => sender.send(Err(Box::new(e))),
                };
            }
            NodeCommand::SimulateNodeFailure { sender, reason } => {
                info!("{} Simulating network failure:", self.nname());
                info!("Triggering heartbeat failure in 500ms: {:?}", reason);
                sender.send(reason.clone()).ok();
                std::thread::sleep(std::time::Duration::from_millis(500));
                self.simulate_heartbeat_failure().await;
            }
            NodeCommand::GetNodeState { sender } => {
                let node_state = self.query_node_state().await;
                sender.send(node_state).ok();
            }
        }
    }
}
