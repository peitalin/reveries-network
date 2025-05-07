use color_eyre::{Result, eyre::anyhow};
use colored::Colorize;
use libp2p::{kad, PeerId};
use tracing::{info, error, warn};

use crate::node_client::NodeCommand;
use crate::types::{
    AgentVesselInfo,
    FragmentRequestEnum,
    FragmentResponseEnum,
    NodeKeysWithVesselStatus,
    VesselStatus,
    PeerIdToNodeStatusKey,
    ReverieKeyfragMessage,
    ReverieMessage,
    ReverieType,
    ReverieKeyfrag,
    ReverieCapsulefrag,
    Reverie,
    ReverieIdToNameKey,
    ReverieIdToPeerId,
    KademliaKeyTrait,
};
use crate::{get_node_name, short_peer_id};
use crate::SendError;
use super::NetworkEvents;


impl NetworkEvents {
    pub(crate) async fn handle_command(&mut self, command: NodeCommand) {
        match command {

            NodeCommand::SendReverieKeyfrag {
                keyfrag_provider, // Key Fragment Provider
                reverie_keyfrag_msg: ReverieKeyfragMessage {
                    reverie_keyfrag,
                    source_peer_id,
                    target_peer_id,
                },
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
                        )
                    );
            }
            NodeCommand::SaveReverieOnNetwork {
                reverie_msg: ReverieMessage {
                    reverie,
                    source_peer_id,
                    target_peer_id,
                    keyfrag_providers,
                }
            } => {
                info!("{}: {} {}",
                    format!("SaveReverieOnNetwork: {:?} {}", reverie.reverie_type, reverie.id).green(),
                    get_node_name(&source_peer_id).yellow(),
                    short_peer_id(&source_peer_id).yellow()
                );

                // 1) Save Agent metadata if need be
                if let ReverieType::SovereignAgent(agent_name_nonce)
                    | ReverieType::Agent(agent_name_nonce) = &reverie.reverie_type {

                    // Set broadcaster's peer info
                    self.peer_manager.set_peer_info_agent_vessel(
                        &AgentVesselInfo {
                            reverie_id: reverie.id.clone(),
                            reverie_type: reverie.reverie_type.clone(),
                            threshold: reverie.threshold,
                            total_frags: reverie.total_frags,
                            current_vessel_peer_id: source_peer_id,
                            next_vessel_peer_id: target_peer_id,
                        }
                    );
                    // Put agent_name_nonce => reverie_id on DHT
                    self.swarm.behaviour_mut().kademlia.put_record(
                        kad::Record {
                            key: agent_name_nonce.to_reverie_id().to_kad_key(),
                            value: serde_json::to_vec(&reverie.id).expect("serde_json::to_vec(reverie_id)"),
                            publisher: Some(self.node_id.peer_id),
                            expires: None,
                        },
                        kad::Quorum::Majority
                    ).expect("put_record err");
                }

                // Dispatch Reverie (ciphertext) to the network
                self.swarm.behaviour_mut().kademlia.put_record(
                    kad::Record {
                        key: kad::RecordKey::new(&reverie.id),
                        value: serde_json::to_vec(
                            &ReverieMessage {
                                reverie,
                                source_peer_id,
                                target_peer_id,
                                keyfrag_providers,
                            }
                        ).expect("serde_json::to_vec(reverie)"),
                        publisher: Some(self.node_id.peer_id),
                        expires: None,
                    },
                    kad::Quorum::Majority
                ).expect("put_record err");
            }
            NodeCommand::SendReverieToSpecificPeer {
                ciphertext_holder, // Ciphertext Holder
                reverie_msg: ReverieMessage {
                    reverie,
                    source_peer_id,
                    target_peer_id,
                    keyfrag_providers,
                },
            } => {
                // 1. Save agent metadata
                if let ReverieType::SovereignAgent(agent_name_nonce)
                    | ReverieType::Agent(agent_name_nonce) = &reverie.reverie_type {

                    // Set broadcaster's peer info
                    self.peer_manager.set_peer_info_agent_vessel(
                        &AgentVesselInfo {
                            reverie_id: reverie.id.clone(),
                            reverie_type: reverie.reverie_type.clone(),
                            threshold: reverie.threshold,
                            total_frags: reverie.total_frags,
                            current_vessel_peer_id: source_peer_id,
                            next_vessel_peer_id: target_peer_id,
                        }
                    );
                    // Put agent_name_nonce => reverie_id on DHT
                    self.swarm.behaviour_mut().kademlia.put_record(
                        kad::Record {
                            key: agent_name_nonce.to_reverie_id().to_kad_key(),
                            value: serde_json::to_vec(&reverie.id).expect("serde_json::to_vec(reverie_id)"),
                            publisher: Some(self.node_id.peer_id),
                            expires: None,
                        },
                        kad::Quorum::Majority
                    ).expect("put_record err");
                }

                // Dispatch Reverie (ciphertext) to target vessel
                self.swarm.behaviour_mut()
                    .request_response
                    .send_request(
                        &ciphertext_holder,
                        FragmentRequestEnum::SaveCiphertextRequest(
                            ReverieMessage {
                                reverie,
                                source_peer_id,
                                target_peer_id,
                                keyfrag_providers,
                            },
                        )
                    );
            }
            NodeCommand::GetReverie {
                reverie_id,
                reverie_type,
                sender,
            } => {
                match reverie_type {
                    ReverieType::SovereignAgent(agent_name_nonce) => {
                        info!("{}", format!("GetReverie({}) from local node", reverie_id).green());
                        // get agent Reverie locally from the node.
                        let reverie = match self.peer_manager.get_reverie(&reverie_id) {
                            Some(reverie) => Ok(reverie.clone()),
                            None => Err(anyhow!("Reverie not found")),
                        };
                        sender.send(reverie).ok();
                    }
                    _ => {
                        info!("{}", format!("GetReverie({}) from DHT Network", reverie_id).green());
                        // Get other Reverie types from the network
                        self.swarm.behaviour_mut()
                            .kademlia
                            .get_record(kad::RecordKey::new(&reverie_id));

                        self.pending.get_reverie_from_network.insert(reverie_id, sender);
                    }
                }
            }
            NodeCommand::GetNodeVesselStatusesFromKademlia { sender } => {

                let peers = self.swarm.connected_peers()
                    .cloned()
                    .collect::<Vec<PeerId>>();

                for peer_id in peers {
                    // add prefix as kademlia key
                    let node_status_kad_key = PeerIdToNodeStatusKey::from(peer_id);
                    self.swarm.behaviour_mut()
                        .kademlia
                        .get_record(node_status_kad_key.to_kad_key());

                    self.pending.get_node_vessels.insert(node_status_kad_key, sender.clone());
                };
            }
            NodeCommand::GetReverieIdByName {
                reverie_name_nonce,
                sender,
            } => {
                // add prefix as kademlia key
                let reverie_to_name_kadkey = ReverieIdToNameKey::from(reverie_name_nonce.clone());

                self.swarm.behaviour_mut()
                    .kademlia
                    .get_record(reverie_to_name_kadkey.to_kad_key());

                self.pending.get_reverie_agent_name.insert(reverie_to_name_kadkey, sender);
            }
            NodeCommand::RequestCapsuleFragment {
                reverie_id,
                kfrag_provider_peer_id,
                access_key,
                sender,
            } => {
                info!("{}", format!("RequestCapsuleFragment for {} from {} {}",
                    reverie_id,
                    get_node_name(&kfrag_provider_peer_id),
                    short_peer_id(&kfrag_provider_peer_id)
                ).yellow());

                let request_id = self.swarm.behaviour_mut()
                    .request_response
                    .send_request(
                        &kfrag_provider_peer_id,
                        FragmentRequestEnum::GetFragmentRequest(
                            reverie_id,
                            access_key
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
                match self.swarm.listen_on(addr.clone()) {
                    Ok(_) => sender.send(Ok(addr.to_string())).ok(),
                    Err(e) => sender.send(Err(Box::new(e))).ok(),
                };
            }
            NodeCommand::GetConnectedPeers { responder } => {
                let connected_peers: Vec<PeerId> = self.swarm.connected_peers().cloned().collect();
                if responder.send(connected_peers.clone()).is_err() {
                    warn!("Failed to send connected peers response. Receiver likely dropped. Peers: {:?}", connected_peers);
                }
            }
            NodeCommand::SimulateNodeFailure { sender, reason } => {
                info!("{}", format!("Simulating network failure: {}", self.nname()).red());
                info!("{}", format!("Triggering heartbeat failure in 500ms: {:?}", reason).red());
                sender.send(reason.clone()).ok();
                std::thread::sleep(std::time::Duration::from_millis(500));
                self.simulate_heartbeat_failure().await;
            }
            NodeCommand::GetNodeState { sender } => {
                let node_state = self.query_node_state().await;
                sender.send(node_state).ok();
            }
            NodeCommand::ReportUsage {
                usage_report,
                sender,
            } => {
                println!("Reporting usage: {:?}", usage_report);
                sender.send(Ok("tx_id".to_string())).ok();
            }
        }
    }
}
