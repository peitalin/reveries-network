use core::str;

use libp2p::{
    kad,
    PeerId,
};
use color_eyre::Result;
use tracing::{info, warn, debug};
use serde::{Deserialize, Serialize};

use crate::{get_node_name, short_peer_id, SendError};
use crate::types::{
    VesselPeerId,
    NodeVesselWithStatus,
    SignedVesselStatus,
    ReverieIdToAgentName,
    ReverieId,
    KademliaKey,
};
use super::NetworkEvents;


impl<'a> NetworkEvents<'a> {

    pub(crate) fn should_dial_peer(&self, peer_id: &PeerId) -> bool {
        // Don't dial:
        // 1. Ourselves
        // 2. Already connected peers
        if peer_id == &self.node_id.peer_id || self.swarm.is_connected(peer_id) {
            debug!("{} Already connected to peer: {:?}", self.nname(), peer_id);
            return false;
        }
        true
    }

    pub(super) async fn handle_kademlia_event(&mut self, kad_event: kad::Event) -> Result<()> {
        match kad_event {
            // Add handling for routing table updates
            kad::Event::RoutingUpdated { peer: peer_id, addresses, .. } => {
                // info!("Kademlia RoutingUpdated: peer={:?}, addresses={:?}", peer_id, addresses);
                if self.should_dial_peer(&peer_id) {
                    self.peer_manager.insert_peer_info(peer_id);
                    for addr in addresses.iter() {
                        // info!("RoutingUpdated: adding Kademlia peer: {:?}", short_peer_id(peer_id));
                        self.swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                    }
                    // Then we try to connect to the newly discovered peer
                    if let Err(e) = self.swarm.dial(peer_id) {
                        warn!("{} Failed to dial peer: {}", short_peer_id(&peer_id), e);
                    }
                }
            },
            kad::Event::OutboundQueryProgressed { id, result, ..} => {
                self.handle_kademlia_query_result(id, result).await?;
            },
            // ignore other Kademlia events
            _kad_event => {}
        }

        Ok(())
    }

    pub(super) async fn handle_kademlia_query_result(
        &mut self,
        query_id: kad::QueryId,
        query_result: kad::QueryResult
    ) -> Result<()> {

        match query_result {

            kad::QueryResult::StartProviding(..) => {}

            kad::QueryResult::GetClosestPeers(Ok(_ok)) => {}

            kad::QueryResult::GetProviders(Ok(p)) => match p {
                kad::GetProvidersOk::FoundProviders { providers, .. } => {
                    if let Some(sender) = self.pending.get_providers.remove(&query_id) {
                        // send providers back
                        sender.send(providers).map_err(SendError::from)?;

                        self.swarm
                            .behaviour_mut()
                            .kademlia
                            .query_mut(&query_id)
                            .unwrap()
                            .finish();
                    }
                }
                kad::GetProvidersOk::FinishedWithNoAdditionalRecord { .. } => {},
            }

            kad::QueryResult::GetRecord(Ok(
                kad::GetRecordOk::FoundRecord(kad::PeerRecord { record, ..  })
            )) => {
                match KademliaKey::from(&record) {
                    KademliaKey::VesselPeerId(vessel_key) => {
                        if let Some(sender) = self.pending.get_node_vessels.remove(&vessel_key) {
                            match serde_json::from_slice::<SignedVesselStatus>(&record.value) {
                                Ok(signed_status) => {
                                    // Verify signature
                                    if let Some(publisher_peer) = record.publisher {
                                        signed_status.verify(&publisher_peer)?;
                                        sender.send(signed_status.node_vessel_status).await?;
                                    } else {
                                        warn!("Record missing publisher ID");
                                    }
                                }
                                Err(e) => warn!("{}", e.to_string()),
                            }
                        }
                    }
                    KademliaKey::ReverieIdToAgentName(reverie_id_to_agent_name_key) => {
                        if let Some(oneshot_sender) = self.pending.get_reverie_agent_name.remove(&reverie_id_to_agent_name_key) {
                            match serde_json::from_slice::<ReverieId>(&record.value) {
                                Ok(reverie_id) => {
                                    oneshot_sender.send(Some(reverie_id)).ok();
                                }
                                Err(e) => {
                                    oneshot_sender.send(None).ok();
                                },
                            }
                        }
                    }
                    KademliaKey::Unknown(s) => {
                        warn!("Unknown Kademlia key: {}", s);
                    }
                }

                // Finish kademlia query
                self.swarm.behaviour_mut()
                    .kademlia
                    .query_mut(&query_id)
                    .unwrap()
                    .finish();
            }

            kad::QueryResult::PutRecord(..) => {}

            kad::QueryResult::Bootstrap(Ok(kad::BootstrapOk { peer: peer_id, .. })) => {
                if peer_id == self.node_id.peer_id {
                    debug!("BootstrapOk: publishing NodeVesselStatus and Umbral PK {:?} {}\n",
                        get_node_name(&peer_id),
                        self.node_id.umbral_key.public_key
                    );

                    let node_vessel_status = NodeVesselWithStatus {
                        peer_id: peer_id,
                        umbral_public_key: self.node_id.umbral_key.public_key,
                        agent_vessel_info: None, // None initially
                        vessel_status: self.peer_manager.vessel_status,
                    };
                    // Publish signed vessel status during bootstrap
                    self.put_signed_vessel_status_kademlia(node_vessel_status)?;
                }
            }

            qresult => debug!("{}: {:?}", self.nname(), qresult)
        }

        Ok(())
    }

}

