use libp2p::{
    kad,
    PeerId
};
use crate::node_client::NodeCommand;
use crate::types::{
    FragmentRequest, FragmentResponse, UmbralPeerId
};
use crate::types::CapsuleFragmentIndexed;
use crate::SendError;

use super::EventLoop;


impl EventLoop {

    pub(crate) async fn handle_command(&mut self, command: NodeCommand) {
        match command {
            NodeCommand::SubscribeTopics { topics, sender } => {
                let subscribed_topics = self.subscribe_topics(topics).await;
                let _ = sender.send(subscribed_topics);
            }
            NodeCommand::UnsubscribeTopics { topics, sender } => {
                let unsubscribed_topics = self.unsubscribe_topics(&topics).await;
                let _ = sender.send(unsubscribed_topics);
            }
            NodeCommand::BroadcastKfrags(key_fragment_message) => {
                let _ = self.broadcast_kfrag(key_fragment_message).await;
            }
            NodeCommand::GetKfragBroadcastPeers { agent_name, agent_nonce, sender } => {
                match self.peer_manager.get_all_kfrag_broadcast_peers(&agent_name, agent_nonce) {
                    None => {
                        println!("missing kfrag_peers: {:?}", self.peer_manager.kfrags_peers);
                        // TODO handle TopicSwitch
                    }
                    Some(peers) => {
                        let _ = sender.send(peers.clone());
                    }
                }
            }
            NodeCommand::RequestCfrags { agent_name, agent_nonce, frag_num, sender } => {

                match self.peer_manager.get_cfrags(&agent_name, &agent_nonce) {
                    None => self.log(format!("No Cfrag peers stored in PeerManager")),
                    Some(cfrag) => {

                        // providers for the kth-frag
                        let providers = self.peer_manager
                            .get_kfrag_broadcast_peers_by_fragment(&agent_name, &agent_nonce, frag_num as u32)
                            // filter out peers that are the vessel node, they created the kfrags
                            .iter()
                            .filter_map(|&peer_id| match peer_id != cfrag.vessel_peer_id {
                                true => Some(peer_id),
                                false => None
                            })
                            .collect::<Vec<PeerId>>();

                        self.log(format!("Located Cfrag broadcast peers: {:?}", providers));
                        if providers.is_empty() {
                            self.log(format!("Could not find provider for agent_name {}", agent_name));
                        } else {

                            if let Some(peer_id) = providers.iter().next() {
                                println!("Requesting Cfrag from peer: {:?}", peer_id);

                                let request_id = self
                                    .swarm
                                    .behaviour_mut()
                                    .request_response
                                    .send_request(
                                        &peer_id,
                                        FragmentRequest(agent_name.clone(), agent_nonce, Some(frag_num as usize))
                                    );

                                self.pending.request_fragments.insert(request_id, sender);
                            }
                        }

                    }
                };
            }
            NodeCommand::GetProviders { agent_name, agent_nonce, sender } => {
                let query_id = self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .get_providers(agent_name.into_bytes().into());

                self.pending.get_providers.insert(query_id, sender);
            }
            NodeCommand::GetPeerUmbralPublicKey { sender, agent_name, agent_nonce } => {

                let public_keys = self.peer_manager
                    .get_connected_peers()
                    .iter()
                    .map(|(peer_id, _peer_info)| {
                        peer_id
                    }).collect::<Vec<&PeerId>>();


                for peer_id in public_keys {

                    let umbral_pk_kademlia_key = UmbralPeerId::from(peer_id);
                    let _query_id = self.swarm.behaviour_mut()
                        .kademlia
                        .get_record(kad::RecordKey::new(&umbral_pk_kademlia_key.to_string()));

                    self.pending.get_umbral_pks.insert(umbral_pk_kademlia_key, sender.clone());
                };
            }
            NodeCommand::RespondCfrag { agent_name, agent_nonce, frag_num, channel } => {

                self.log(format!("RespondCfrags: Finding topic for: {agent_name}-{agent_nonce}"));

                let cfrag_indexed = match self.peer_manager.get_cfrags(&agent_name, &agent_nonce) {
                    None => None,
                    Some(cfrag) => {
                        self.log(format!("RespondCfrags: Found Cfrag: {:?}", cfrag));

                        let cfrag_indexed_bytes = serde_json::to_vec::<Option<CapsuleFragmentIndexed>>(&Some(cfrag.clone()))
                            .map_err(|e| SendError(e.to_string()));

                        // Return None if peer does not have the cfrag
                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_response(channel, FragmentResponse(cfrag_indexed_bytes))
                            .expect("Connection to peer to be still open.");

                        Some(cfrag)
                    }
                };
                //// TODO: handle error properly
                // Do not send if cfrag not found. Handle futures error

            }
            NodeCommand::StartListening { addr, sender } => {
                let _ = match self.swarm.listen_on(addr) {
                    Ok(_) => sender.send(Ok(())),
                    Err(e) => sender.send(Err(Box::new(e))),
                };
            }
            NodeCommand::RequestFile {
                agent_name,
                agent_nonce,
                frag_num,
                peer,
                sender,
            } => {
                let request_id = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer, FragmentRequest(agent_name, agent_nonce, frag_num));

                self.pending.request_fragments.insert(request_id, sender);
            }
            NodeCommand::RespondFile { file, channel } => {
                self.swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(channel, FragmentResponse(Ok(file)))
                    .expect("Connection to peer to be still open.");
            }
        }

    }
}