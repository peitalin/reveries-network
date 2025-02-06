use colored::Colorize;
use color_eyre::owo_colors::OwoColorize;
use libp2p::Multiaddr;
use libp2p::{
    kad, mdns, swarm::SwarmEvent
};
use crate::behaviour::BehaviourEvent;
use crate::{short_peer_id, get_node_name};
use crate::types::{
    UmbralPeerId,
    UmbralPublicKeyResponse,
};
use super::EventLoop;


impl EventLoop {

    pub(super) async fn handle_swarm_event(&mut self, event: SwarmEvent<BehaviourEvent>) {
        match event {

            //// Kademlia Protocol tracks:
            //// - Umbral Public Keys (Proxy Reencryption) for each PeerId this node is connected to.
            //// - Ciphertexts
            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(e)) => match e {

                // StartProviding event
                kad::Event::OutboundQueryProgressed {
                    result: kad::QueryResult::StartProviding(_), ..
                } => {}

                // GetProviders event
                kad::Event::OutboundQueryProgressed { id, result, ..} => match result {
                    kad::QueryResult::GetProviders(p) => match p {
                        Ok(kad::GetProvidersOk::FoundProviders { key, providers }) => {

                            for peer in providers.clone() {
                                let kkey = std::str::from_utf8(key.as_ref()).unwrap();
                                self.log(format!("{peer:?} provides key {:?}", kkey));
                            }

                            if let Some(sender) = self.pending.get_providers.remove(&id) {
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
                        let umbral_pk_peer_id_key: UmbralPeerId = k.into();

                        if let Some(sender) = self.pending.get_umbral_pks.remove(&umbral_pk_peer_id_key) {

                            match serde_json::from_slice::<UmbralPublicKeyResponse>(&value) {
                                Ok(umbral_pk_response) => {
                                    let _ = sender.send(umbral_pk_response).await;
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
                    kad::QueryResult::GetRecord(Ok(_r)) => {
                        // self.log(format!("GetRecord: {:?}", r));
                    }
                    kad::QueryResult::GetRecord(Err(_err)) => {
                        // self.log(format!("Failed to get record {err:?}"));
                    }
                    kad::QueryResult::PutRecord(Ok(kad::PutRecordOk { key })) => {
                        // let r = std::str::from_utf8(key.as_ref()).unwrap();
                        // self.log(format!("PutRecordOk {:?}", r));
                    }
                    kad::QueryResult::PutRecord(Err(_err)) => {
                        // self.log(format!("Failed to PutRecord: {err:?}"));
                    }
                    kad::QueryResult::StartProviding(Ok(kad::AddProviderOk { .. })) => {
                        // let r = std::str::from_utf8(key.as_ref()).unwrap();
                        // self.log(format!("StartProviding: {:?}", r));
                    }
                    kad::QueryResult::StartProviding(Err(_err)) => {
                        // self.log(format!("Failed to StartProviding: {err:?}"));
                    }
                    kad::QueryResult::Bootstrap(Ok(kad::BootstrapOk { peer, .. })) => {

                        if peer == self.peer_id {

                            self.log(format!(
                                "Kademlia BootstrapOk: Publishing Umbral PK for {:?} {}\n",
                                get_node_name(&peer),
                                self.umbral_key.public_key
                            ));

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
                }
                _ => {} // ignore other Kademlia events
            }

            //// mDNS Protocol
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                for (peer_id, multiaddr) in list {
                    self.log(format!("mDNS adding peer {:?}", peer_id));
                    self.swarm.behaviour_mut().kademlia.add_address(&peer_id, multiaddr);
                    self.swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    self.peer_manager.insert_peer_info(peer_id);
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                for (peer_id, multiaddr) in list {
                    self.log(format!("mDNS peer expired {:?}. Removing peer.", peer_id));
                    self.swarm.behaviour_mut().kademlia.remove_address(&peer_id, &multiaddr);
                }
            },

            //// Request Response Protocol
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                request_response_event
            )) => {
                self.handle_request_response(request_response_event).await
            },

            //// Connections
            SwarmEvent::NewListenAddr { .. } => {}
            SwarmEvent::Dialing { .. } => {}
            SwarmEvent::IncomingConnection { .. } => {},
            SwarmEvent::ConnectionEstablished { .. } => { }
            SwarmEvent::ExpiredListenAddr { .. } => { }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                println!(">>> ConnectionClosed with peer: {:?}", peer_id);
                self.remove_peer(&peer_id);
            }
            SwarmEvent::IncomingConnectionError { .. } => { }
            SwarmEvent::OutgoingConnectionError { .. } => { }

            //////////////////////////////////
            //// GossipSub protocol for PRE
            //////////////////////////////////
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gevent)) => {
                self.handle_gossipsub_event(gevent).await;
            }

            SwarmEvent::Behaviour(BehaviourEvent::Heartbeat(tee_event)) => {
                self.peer_manager.update_peer_heartbeat(tee_event.peer_id, tee_event.latest_tee_attestation);
                if let Some(tee_str) = self.peer_manager.log_heartbeat_tee(tee_event.peer_id) {
                    self.log(tee_str);
                }
            }

            swarm_event => println!("Unhandled SwarmEvent: {swarm_event:?}"),
        }
    }

}

