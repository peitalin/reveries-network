use libp2p::{
    kad,
    Multiaddr,
    PeerId,
};
use libp2p_kad::GetClosestPeersOk;
use tracing::{info, warn, debug};
use crate::get_node_name;
use crate::types::{
    UmbralPeerId,
    UmbralPublicKeyResponse,
};
use super::NetworkEvents;


//// Kademlia only used for tracking Umbral PublicKeys (Proxy Reencryption) for each PeerId
impl<'a> NetworkEvents<'a> {

    pub(crate) fn should_dial_peer(&self, peer_id: &PeerId) -> bool {
        // Don't dial:
        // 1. Ourselves
        // 2. Already connected peers
        if peer_id == &self.peer_id || self.swarm.is_connected(peer_id) {
            debug!("{} Already connected to peer: {:?}", self.nname(), peer_id);
            return false;
        }
        true
    }

    pub(super) async fn handle_kademlia_event(&mut self, kad_event: kad::Event) {
        match kad_event {
            // GetProviders event
            kad::Event::OutboundQueryProgressed { id, result, ..} => match result {

                kad::QueryResult::StartProviding(..) => {}
                kad::QueryResult::GetClosestPeers(Ok(_ok)) => {}
                kad::QueryResult::GetProviders(Ok(p)) => match p {
                    kad::GetProvidersOk::FoundProviders { providers, .. } => {
                        if let Some(sender) = self.pending.get_providers.remove(&id) {
                            // send providers back
                            sender.send(providers).expect("Receiver not to be dropped");
                            self.swarm
                                .behaviour_mut()
                                .kademlia
                                .query_mut(&id)
                                .unwrap()
                                .finish();
                        }
                    }
                    kad::GetProvidersOk::FinishedWithNoAdditionalRecord { .. } => {},
                }
                kad::QueryResult::GetRecord(Ok(
                    kad::GetRecordOk::FoundRecord(kad::PeerRecord { record, ..  })
                )) => {
                    let k = std::str::from_utf8(record.key.as_ref()).unwrap();

                    // Handle Umbral keys
                    if let Ok(umbral_pk_key) = UmbralPeerId::from_string(k) {
                        if let Some(sender) = self.pending.get_umbral_pks.remove(&umbral_pk_key) {
                            match serde_json::from_slice::<UmbralPublicKeyResponse>(&record.value) {
                                Ok(umbral_pk_response) => {
                                    sender.send(umbral_pk_response).await.ok();
                                }
                                Err(_e) => println!("Err deserializing UmbralPublicKeyResponse"),
                            }
                            self.swarm
                                .behaviour_mut()
                                .kademlia
                                .query_mut(&id)
                                .unwrap()
                                .finish();
                        }
                    }
                }
                kad::QueryResult::GetRecord(..) => {}
                kad::QueryResult::PutRecord(..) => {}
                kad::QueryResult::Bootstrap(Ok(kad::BootstrapOk { peer, .. })) => {
                    if peer == self.peer_id {
                        debug!(
                            "Kademlia BootstrapOk: Publishing Umbral PK for {:?} {}\n",
                            get_node_name(&peer),
                            self.umbral_key.public_key
                        );

                        let umbral_pk_response = UmbralPublicKeyResponse {
                            umbral_peer_id: UmbralPeerId::from(peer),
                            umbral_public_key: self.umbral_key.public_key,
                        };
                        let umbral_public_key_bytes = serde_json::to_vec(&umbral_pk_response)
                            .expect("serializing Umbral PRE key");

                        self.swarm
                            .behaviour_mut()
                            .kademlia
                            .put_record(
                                kad::Record {
                                    key: kad::RecordKey::new(&umbral_pk_response.umbral_peer_id.to_string()),
                                    value: umbral_public_key_bytes,
                                    publisher: Some(peer),
                                    expires: None,
                                },
                                kad::Quorum::One
                            ).expect("No store error.");
                    }
                }
                qresult => println!("<NetworkEvent>: {:?}", qresult)
            },
            // Add handling for routing table updates
            kad::Event::RoutingUpdated { peer: peer_id, addresses, .. } => {
                if self.should_dial_peer(&peer_id) {
                    self.peer_manager.insert_peer_info(peer_id);
                    for addr in addresses.iter() {
                        info!("RoutingUpdated: adding Kademlia peer: {:?}", addr);  // This shows us learning about other peers
                        self.swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                    }
                    // Then we try to connect to the newly discovered peer
                    if let Err(e) = self.swarm.dial(peer_id) {
                        warn!("{} Failed to dial discovered peer: {}", self.nname(), e);
                    }
                }
            },

            // ignore other Kademlia events
            _kad_event => {}
        }
    }
}

