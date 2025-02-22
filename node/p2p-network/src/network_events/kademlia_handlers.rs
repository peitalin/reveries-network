use libp2p::kad;
use tracing::{info, warn, debug};
use crate::get_node_name;
use crate::types::{
    UmbralPeerId,
    UmbralPublicKeyResponse,
};
use super::NetworkEvents;


//// Kademlia only used for tracking Umbral PublicKeys (Proxy Reencryption) for each PeerId
impl<'a> NetworkEvents<'a> {
    pub(super) async fn handle_kademlia_event(&mut self, kad_event: kad::Event) {
        match kad_event {
            // GetProviders event
            kad::Event::OutboundQueryProgressed { id, result, ..} => match result {
                kad::QueryResult::GetProviders(p) => match p {
                    Ok(kad::GetProvidersOk::FoundProviders { providers, .. }) => {
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
                    kad::GetRecordOk::FoundRecord(kad::PeerRecord {
                        record: kad::Record { key, value, .. },
                        ..
                    })
                )) => {
                    let k = std::str::from_utf8(key.as_ref()).unwrap();
                    let umbral_pk_peer_id_key: UmbralPeerId = k.into();

                    if let Some(sender) = self.pending.get_umbral_pks.remove(&umbral_pk_peer_id_key) {
                        match serde_json::from_slice::<UmbralPublicKeyResponse>(&value) {
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
                qresult => println!("<NetworkEvent>: {:?}", qresult)
            },
            // Add handling for routing table updates
            kad::Event::RoutingUpdated { peer, addresses, .. } => {
                debug!("{} {}", self.nname(), format!("Kademlia discovered peer {:?} at {:?}", peer, addresses));
                // Add or update peers as they're discovered through the DHT
                for addr in addresses.iter() {
                    self.swarm.behaviour_mut().kademlia.add_address(&peer, addr.clone());
                }
            },

            // ignore other Kademlia events
            _kad_event => {}
        }
    }
}

