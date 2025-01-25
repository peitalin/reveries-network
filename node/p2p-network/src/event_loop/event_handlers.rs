use futures::prelude::*;
use libp2p::{
    kad, mdns, request_response, swarm::SwarmEvent
};
use crate::behaviour::{
    BehaviourEvent,
    FileEvent,
    UmbralPeerId,
    UmbralPublicKeyResponse,
};
use crate::short_peer_id;
use super::EventLoop;


impl EventLoop {

    pub(super) async fn handle_event(&mut self, event: SwarmEvent<BehaviourEvent>) {
        match event {

            //// Kademlia Protocol tracks:
            //// - which peers hold which file_names
            //// - Umbral Public Keys (Proxy Reencryption) for each PeerId this node is connected to.
            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(e)) => match e {

                // StartProviding event
                kad::Event::OutboundQueryProgressed {
                    id, result: kad::QueryResult::StartProviding(_), ..
                } => {}

                // GetProviders event
                kad::Event::OutboundQueryProgressed { id, result, ..} => match result {
                    kad::QueryResult::GetProviders(p) => match p {
                        Ok(kad::GetProvidersOk::FoundProviders { key, providers }) => {

                            for peer in providers.clone() {
                                let kkey = std::str::from_utf8(key.as_ref()).unwrap();
                                self.log(format!("{peer:?} provides key {:?}", kkey));
                            }

                            if let Some(sender) = self.pending_get_providers.remove(&id) {
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
                            self.log(format!("Failed to get providers: {err:?}"));
                        }
                    }
                    kad::QueryResult::GetRecord(Ok(
                        kad::GetRecordOk::FoundRecord(kad::PeerRecord {
                            record: kad::Record { key, value, .. },
                            ..
                        })
                    )) => {
                        let k = std::str::from_utf8(key.as_ref()).unwrap();
                        // let v = std::str::from_utf8(&value).unwrap();
                        // self.log(format!("Got record {:?} {:?}", k, v));
                        let umbral_pk_peer_id_key: UmbralPeerId = k.into();

                        if let Some(sender) = self.pending_get_umbral_pks.remove(&umbral_pk_peer_id_key) {

                            let umbral_pk_response = serde_json::from_slice::<UmbralPublicKeyResponse>(&value)
                                .expect("err deserializing Umbral PRE Public Key");

                            // sender.send(umbral_pk_response).expect("Receiver not to be dropped");
                            let _ = sender.send(umbral_pk_response).await;
                            // Finish the query. We are only interested in the first result.
                            self.swarm
                                .behaviour_mut()
                                .kademlia
                                .query_mut(&id)
                                .unwrap()
                                .finish();
                        }
                    }
                    kad::QueryResult::GetRecord(Ok(r)) => {
                        // self.log(format!("GetRecord: {:?}", r));
                    }
                    kad::QueryResult::GetRecord(Err(err)) => {
                        // self.log(format!("Failed to get record {err:?}"));
                    }
                    kad::QueryResult::PutRecord(Ok(kad::PutRecordOk { key })) => {
                        let r = std::str::from_utf8(key.as_ref()).unwrap();
                        // self.log(format!("PutRecordOk {:?}", r));
                    }
                    kad::QueryResult::PutRecord(Err(err)) => {
                        self.log(format!("Failed to PutRecord: {err:?}"));
                    }
                    kad::QueryResult::StartProviding(Ok(kad::AddProviderOk { key })) => {
                        let r = std::str::from_utf8(key.as_ref()).unwrap();
                        self.log(format!("StartProviding: {:?}", r));
                    }
                    kad::QueryResult::StartProviding(Err(err)) => {
                        // self.log(format!("Failed to StartProviding: {err:?}"));
                    }
                    kad::QueryResult::Bootstrap(Ok(kad::BootstrapOk { peer, .. })) => {

                        if peer == self.peer_id {
                            self.log(format!("BootstrapOk: Publishing Umbral PK for {:?} {}", peer, self.umbral_key.public_key));
                            let umbral_pk_response = UmbralPublicKeyResponse {
                                peer_id: UmbralPeerId::from(peer),
                                umbral_public_key: self.umbral_key.public_key,
                            };
                            let umbral_public_key_bytes = serde_json::to_vec(&umbral_pk_response)
                                .expect("serializing Umbral PRE key");

                            self.swarm
                                .behaviour_mut()
                                .kademlia
                                .put_record(
                                    kad::Record {
                                        key: kad::RecordKey::new(&umbral_pk_response.peer_id.to_string()),
                                        value: umbral_public_key_bytes,
                                        publisher: Some(peer),
                                        expires: None,
                                    },
                                    kad::Quorum::One
                                ).expect("No store error.");
                        }
                    }
                    qresult => println!("<NetworkEvent>: {:?}", qresult)
                }
                _ => {} // ignore other Kademlia events
            }

            //// Request Response Protocol
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                request_response::Event::Message { message, .. }
            )) => {
                match message {
                    request_response::Message::Request {
                        request,
                        channel,
                        ..
                    } => {
                        self.network_event_sender
                            .send(FileEvent::InboundRequest {
                                request: request.0,
                                frag_num: request.1,
                                channel,
                            })
                            .await
                            .expect("Event receiver not to be dropped.");
                    }
                    request_response::Message::Response {
                        request_id,
                        response,
                    } => {
                        let _ = self
                            .pending_request_file
                            .remove(&request_id)
                            .expect("Request pending.")
                            .send(Ok(response.0));
                    }
                }
            },
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                request_response::Event::InboundFailure { .. },
            )) => {
                // self.log(format!("InboundFailure: {:?} {:?} {:?}", peer, request_id, error));
            }
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                request_response::Event::OutboundFailure {
                    request_id, error, peer
                },
            )) => {
                // self.log(format!("OutboundFailure: {:?} {:?} {:?}", peer, request_id, error));
                let _ = self
                    .pending_request_file
                    .remove(&request_id)
                    .expect("Request pending")
                    .send(Err(Box::new(error)));

            }
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                request_response::Event::ResponseSent { peer, .. },
            )) => {
                self.log(format!("ResponseSent to {:?}", peer));
            }

            //// mDNS Protocol
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                for (peer_id, multiaddr) in list {
                    // self.log(format!("mDNS adding peer {:?}", peer_id));
                    self.swarm.behaviour_mut().kademlia.add_address(&peer_id, multiaddr);
                    self.swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                for (peer_id, multiaddr) in list {
                    // self.log(format!("mDNS peer expired {:?}. Removing peer.", peer_id));
                    self.swarm.behaviour_mut().kademlia.remove_address(&peer_id, &multiaddr);
                    self.swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                }
            },

            //// Connections
            SwarmEvent::NewListenAddr { .. } => {}
            SwarmEvent::IncomingConnection { .. } => {},
            SwarmEvent::ConnectionEstablished { .. } => {}
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                self.swarm
                    .behaviour_mut()
                    .kademlia
                    .remove_record(
                        &kad::RecordKey::new(&UmbralPeerId::from(peer_id).to_string()),
                    );
            }
            SwarmEvent::OutgoingConnectionError { .. } => {}
            SwarmEvent::IncomingConnectionError { .. } => {}
            SwarmEvent::ExpiredListenAddr { .. } => {}
            SwarmEvent::Dialing { .. } => {}

            //////////////////////////////////
            //// GossipSub protocol for PRE
            //////////////////////////////////
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gevent)) => {
                self.handle_gossipsub_event(gevent).await;
            }

            SwarmEvent::Behaviour(BehaviourEvent::Heartbeat(tee_event)) => {

                self.peer_manager.update_heartbeat(
                    tee_event.peer_id,
                    tee_event.latest_tee_attestation.clone()
                );

                let peer_id = &tee_event.peer_id;

                // need to save heartbeat data to the node locally for quicker retrieval;
                let p_info = self.peer_manager.vessel_nodes.get(peer_id);

                match p_info {
                    Some(peer_info) => {
                        match &peer_info.peer_heartbeat_data.heartbeat_payload.tee_attestation {
                            Some(quote) => {
                                self.log(format!(
                                    "{} HeartbeatData: Block: {}\n\tECDSA attestation key: 0x{}",
                                    short_peer_id(peer_id),
                                    peer_info.peer_heartbeat_data.heartbeat_payload.block_height,
                                    hex::encode(quote.signature.ecdsa_attestation_key),
                                ));
                            },
                            None => {
                                self.log(format!(
                                    "{} HeartbeatData: Block: {}\n\t{:?}",
                                    short_peer_id(peer_id),
                                    peer_info.peer_heartbeat_data.heartbeat_payload.block_height,
                                    peer_info.peer_heartbeat_data.heartbeat_payload
                                ));
                            }
                        };
                    }
                    None => {
                        self.log(format!(
                            "HeartbeatData for: {} not found",
                            short_peer_id(peer_id),
                        ));
                    }
                }

            }

            e => println!("??? Some SwarmEvent: {e:?}"),
        }
    }

}

