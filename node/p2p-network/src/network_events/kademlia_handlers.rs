use libp2p::{
    kad,
    Multiaddr,
    PeerId,
};
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
        // Don't dial ourselves
        if peer_id == &self.peer_id {
            return false;
        }

        // Check if we're already connected or dialing
        !self.swarm.is_connected(peer_id)
        // && !self.swarm.network_info().peer_info(peer_id)
        //     .map(|info| info.is_dialing())
        //     .unwrap_or(false)
    }

    pub(super) async fn handle_kademlia_event(&mut self, kad_event: kad::Event) {
        match kad_event {
            // GetProviders event
            kad::Event::OutboundQueryProgressed { id, result, ..} => match result {
                kad::QueryResult::GetProviders(p) => match p {
                    Ok(kad::GetProvidersOk::FoundProviders { providers, .. }) => {
                        info!("{} Found providers: {:?}", self.nname(), providers);
                        if let Some(sender) = self.pending.get_providers.remove(&id) {
                            // send providers back
                            sender.send(providers).expect("Receiver not to be dropped");
                            // Finish the query. We are only interested in the first result.
                            self.swarm
                                .behaviour_mut()
                                .kademlia
                                .query_mut(&id)
                                .unwrap()
                                .finish();
                        }
                    }
                    Ok(kad::GetProvidersOk::FinishedWithNoAdditionalRecord { .. }) => {},
                    Err(err) => {
                        warn!("Failed to get providers: {err:?}");
                    }
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

                    // Handle peer records
                    if k.starts_with("peers:") {
                        if let Ok(peer_list) = serde_json::from_slice::<Vec<PeerId>>(&record.value) {
                            info!("{} Found peer list in DHT: {:?}", self.nname(), peer_list);
                            for peer_id in peer_list {
                                // Add to peer manager
                                self.peer_manager.insert_peer_info(peer_id);

                                // Try to connect
                                if self.should_dial_peer(&peer_id) {
                                    if let Err(e) = self.swarm.dial(peer_id) {
                                        warn!("{} Failed to dial peer from DHT record: {}", self.nname(), e);
                                    }
                                }

                                // Query for their addresses
                                self.swarm.behaviour_mut().kademlia.get_providers(
                                    kad::RecordKey::new(&peer_id.to_string())
                                );
                            }
                        }
                    }

                    // Handle address records
                    if let Some(peer_id) = record.publisher {
                        if let Ok(addrs) = serde_json::from_slice::<Vec<Multiaddr>>(&record.value) {
                            for addr in addrs {
                                self.swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                            }
                            // Only dial if we're not already connected or dialing
                            if self.should_dial_peer(&peer_id) {
                                if let Err(e) = self.swarm.dial(peer_id) {
                                    warn!("{} Failed to dial peer with found addresses: {}", self.nname(), e);
                                }
                            }
                        }
                    }
                }
                kad::QueryResult::GetRecord(..) => {}
                kad::QueryResult::PutRecord(..) => {}
                kad::QueryResult::StartProviding(..) => {}
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
                kad::QueryResult::GetClosestPeers(Ok(ok)) => {
                    info!("{} Found closest peers: {:?}", self.nname(), ok.peers);

                    // Try to connect to each discovered peer
                    for peer_info in &ok.peers {
                        let peer_id = peer_info.peer_id;

                        // Try to connect
                        if self.should_dial_peer(&peer_id) {
                            if let Err(e) = self.swarm.dial(peer_id) {
                                warn!("{} Failed to dial discovered peer: {}", self.nname(), e);
                            }
                        }
                    }
                }
                qresult => println!("<NetworkEvent>: {:?}", qresult)
            },
            // Add handling for routing table updates
            kad::Event::RoutingUpdated { peer, addresses, .. } => {
                info!("{} {}", self.nname(), format!("Kademlia discovered peer {:?} at {:?}", peer, addresses));

                let connected = self.swarm.connected_peers().collect::<Vec<_>>();
                info!("{} Currently connected to: {:?}", self.nname(), connected);

                // Add all discovered addresses
                for addr in addresses.iter() {
                    self.swarm.behaviour_mut().kademlia.add_address(&peer, addr.clone());
                }

                // Add to peer manager when discovered
                self.peer_manager.insert_peer_info(peer);

                // Only dial if we're not already connected or dialing
                if self.should_dial_peer(&peer) {
                    info!("{} Attempting to dial newly discovered peer: {:?}", self.nname(), peer);
                    if let Err(e) = self.swarm.dial(peer) {
                        warn!("{} Failed to dial discovered peer: {}", self.nname(), e);
                    }
                } else {
                    info!("{} Already connected to peer: {:?}", self.nname(), peer);
                }

                // Request their routing table
                self.swarm.behaviour_mut().kademlia.get_closest_peers(peer);

                // Also try to get their stored records
                let key = kad::RecordKey::new(&format!("peers:{}", peer));
                self.swarm.behaviour_mut().kademlia.get_record(key);

                // And look for any providers they know about
                self.swarm.behaviour_mut().kademlia.get_providers(kad::RecordKey::new(&peer.to_string()));
            },

            // ignore other Kademlia events
            _kad_event => {}
        }
    }
}

