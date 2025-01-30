mod chat_handlers;
mod event_handlers;
mod gossipsub_handlers;
pub(crate) mod heartbeat_behaviour;
mod peer_manager;

use std::{
    collections::{HashMap, HashSet},
    error::Error,
};
use color_eyre::Result;
use colored::Colorize;
use futures::{
    // channel::{mpsc, oneshot},
    Stream,
    StreamExt,
    FutureExt,
};
use tokio::sync::{mpsc, oneshot};
use libp2p::{
    gossipsub::IdentTopic,
    kad,
    request_response,
    swarm::Swarm,
    PeerId
};
use runtime::reencrypt::UmbralKey;
use crate::{commands::NodeCommand, get_node_name, AgentName};
use crate::types::{
    ChatMessage,
    UmbralPeerId,
};
use crate::behaviour::{
    Behaviour,
    CapsuleFragmentIndexed,
    FileEvent,
    FragmentRequest,
    FragmentResponse,
    KeyFragmentMessage,
    KfragsTopic,
    UmbralPublicKeyResponse
};
use peer_manager::PeerManager;
use tokio::time;
use time::Duration;
use umbral_pre::VerifiedCapsuleFrag;
use runtime::llm::{AgentSecretsJson, test_claude_query};


pub struct EventLoop {

    swarm: Swarm<Behaviour>,
    command_receiver: mpsc::Receiver<NodeCommand>,
    network_event_sender: mpsc::Sender<FileEvent>,

    // My node's PeerInfo
    peer_id: PeerId,
    node_name: String,

    // chat
    chat_receiver: mpsc::Receiver<ChatMessage>,
    topics: HashMap<String, IdentTopic>,

    // Umbral fragments
    umbral_key: UmbralKey,
    cfrags: HashMap<AgentName, CapsuleFragmentIndexed>,

    // Uses a different channel than command_receiver
    kfrags_receiver: mpsc::Receiver<KeyFragmentMessage>,

    // tracks peer heartbeats status
    peer_manager: PeerManager,

    // pending P2p network requests
    pending: PendingRequests,

    interval: time::Interval,
    heartbeat_failure_receiver: tokio::sync::mpsc::Receiver<String>,
}

struct PendingRequests {
    get_providers: HashMap<
        kad::QueryId,
        oneshot::Sender<HashSet<PeerId>>
    >,
    get_umbral_pks: HashMap<
        UmbralPeerId,
        tokio::sync::mpsc::Sender<UmbralPublicKeyResponse>
    >,
    request_fragments: HashMap<
        request_response::OutboundRequestId,
        oneshot::Sender<Result<Vec<u8>, Box<dyn Error + Send>>>
    >,
}

impl PendingRequests {
    fn new() -> Self {
        Self {
            get_providers: Default::default(),
            get_umbral_pks: Default::default(),
            request_fragments: Default::default(),
        }
    }
}

impl EventLoop {

    pub fn new(
        peer_id: PeerId,
        swarm: Swarm<Behaviour>,
        command_receiver: mpsc::Receiver<NodeCommand>,
        network_event_sender: mpsc::Sender<FileEvent>,
        chat_receiver: mpsc::Receiver<ChatMessage>,
        node_name: String,
        kfrags_receiver: mpsc::Receiver<KeyFragmentMessage>,
        umbral_key: UmbralKey,
        heartbeat_failure_receiver: tokio::sync::mpsc::Receiver<String>,
    ) -> Self {

        Self {
            peer_id,
            swarm,
            command_receiver,
            network_event_sender,
            chat_receiver,
            node_name,
            topics: HashMap::new(),
            cfrags: HashMap::new(),
            kfrags_receiver,
            umbral_key: umbral_key,
            peer_manager: PeerManager::new(),
            pending: PendingRequests::new(),
            interval: tokio::time::interval(Duration::from_secs(4)),
            heartbeat_failure_receiver,
        }
    }

    fn log<S: std::fmt::Display>(&self, message: S) {
        println!("{} {}{} {}",
            "NetworkEvent".green(), self.node_name.yellow(), ">".blue(),
            message
        );
    }

    pub async fn listen_for_commands_and_events(mut self) {

        loop {
            tokio::select! {
                _instant = self.interval.tick() => {
                    // Replace interval with consensus that tracks block height,
                    // order of transactions, state roots from executing state transitions
                    // in the REVM
                    self.swarm.behaviour_mut().heartbeat.increment_block_height();

                    let mut failed_agents = vec![];

                    // iterate and check which peers have stopped sending heartbeats
                    for (peer_id, peer_info) in self.peer_manager.vessel_nodes.iter() {

                        let last_hb = peer_info.peer_heartbeat_data.last_heartbeat;
                        let duration = peer_info.peer_heartbeat_data.duration_since_last_heartbeat();
                        let node_name = get_node_name(&peer_id).magenta();
                        // println!(">>>>last heartbeat: {:?}", last_hb);
                        println!("{} last seen {:?} seconds ago", node_name, duration);

                        if duration > Duration::from_millis(5_000) {
                            if let Some(agent_name) = &peer_info.hosting_agent_name {

                                self.log(format!(
                                    "{} died. Voting to reincarnate agent {}",
                                    node_name,
                                    agent_name
                                ).magenta());

                                // TODO: consensus mechanism to vote for reincarnation
                                failed_agents.push(agent_name);
                            } else {
                                self.log(format!("{} died but wasn't hosting agent.", node_name).purple());
                            }
                        }
                    };

                    if let Some(agent_name) = failed_agents.iter().next() {
                        // let topic = KfragsTopic::RequestCfrags(agent_name.to_string());
                        // self.request_cfrags(topic).await;
                        // self.get_cfrags2(agent_name.to_string()).await;
                    }

                }
                swarm_event = self.swarm.select_next_some() => self.handle_swarm_event(swarm_event).await,
                heartbeat = self.heartbeat_failure_receiver.recv() => match heartbeat {
                    Some(hb) => self.handle_heartbeat_failure(hb).await,
                    None => break // channel closed, shutting down the network event loop.
                },
                command = self.command_receiver.recv() => match command {
                    Some(c) => self.handle_command(c).await,
                    None => return
                },
                chat_message = self.chat_receiver.recv() => match chat_message {
                    Some(cm) => self.broadcast_chat_message(cm).await,
                    None => return
                },
                // remove this and use command channel later
                kfrags_message = self.kfrags_receiver.recv() => match kfrags_message {
                    Some(cm) => match cm.topic {
                        KfragsTopic::BroadcastKfrag(..) => {
                            self.broadcast_kfrag(cm).await;
                        },
                        _ => {} // only for broadcasting kfrags
                    }
                    None => return
                },
            }
        }
    }

    async fn handle_heartbeat_failure(&mut self, heartbeat: String) {
        self.log(format!("{}", heartbeat));
        println!("\tTodo: initiating LLM runtime shutdown...");
        println!("\tTodo: attempt to broadcast agent_secrets reencryption fragments...");
        // Shutdown the LLM runtime (if in Vessel Mode), but
        // continue attempting to broadcast the agent_secrets reencryption fragments and ciphertexts.
        //
        // If the node never reconnects to the network, then 1up-network nodes will form consensus that
        // the Vessel is dead, and begin reincarnating the Agent from it's last public agent_secret ciphertexts
        // on the Kademlia network.
    }

    async fn handle_command(&mut self, command: NodeCommand) {
        match command {
            NodeCommand::SubscribeTopics { topics, sender } => {
                let subscribed_topics = self.subscribe_topics(&topics).await;
                let _ = sender.send(subscribed_topics);
            }
            NodeCommand::UnsubscribeTopics { topics, sender } => {
                let unsubscribed_topics = self.unsubscribe_topics(&topics).await;
                let _ = sender.send(unsubscribed_topics);
            }
            NodeCommand::BroadcastKfrags(KeyFragmentMessage {
                topic,
                frag_num,
                threshold,
                alice_pk,
                bob_pk,
                verifying_pk,
                // TODO: split data into private data for the MPC node, vs public data for kademlia
                // private: kfrags, verify_pk, alice_pk, bob_pk -> store within the MPC node
                // public: capsules and ciphertexts -> store on Kademlia
                vessel_peer_id,
                kfrag,
                capsule,
                ciphertext
            }) => {

            }
            NodeCommand::GetKfragPeers { agent_name, sender } => {
                let peers = self.peer_manager
                    .get_all_umbral_kfrag_providers(&agent_name)
                    .expect("kfrag peers missing")
                    .clone();

                let _ = sender.send(peers);
            }
            NodeCommand::RequestCfrags { agent_name, frag_num, sender } => {

                match self.cfrags.get(&agent_name) {
                    None => {},
                    Some(cfrag) => {

                        // providers for the kth-frag
                        let providers = self.peer_manager
                            .get_umbral_kfrag_providers(&agent_name, frag_num as u32)
                            // filter out peers that are the vessel node, they created the kfrags
                            .iter()
                            .filter_map(|&peer_id| match peer_id != cfrag.vessel_peer_id {
                                true => Some(peer_id),
                                false => None
                            })
                            .collect::<Vec<PeerId>>();

                        self.log(format!("Located Cfrag broadcast peers: {:?}\n", providers));
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
                                        FragmentRequest(agent_name.clone(), Some(frag_num as usize))
                                    );

                                self.pending.request_fragments.insert(request_id, sender);
                            }
                        }

                    }
                };
            }
            NodeCommand::GetProviders { agent_name, sender } => {
                let query_id = self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .get_providers(agent_name.into_bytes().into());

                self.pending.get_providers.insert(query_id, sender);
            }
            NodeCommand::GetPeerUmbralPublicKeys { sender } => {

                // get connected peers, and request their Umbral PKs
                let peer_ids = self.swarm
                    .connected_peers()
                    .map(|&peer_id| peer_id.clone())
                    .collect::<Vec<PeerId>>();

                for peer_id in peer_ids {

                    let umbral_pk_peer_id: UmbralPeerId = peer_id.into();
                    let umbral_pk_peer_id_key: String = umbral_pk_peer_id.clone().into();

                    let _query_id = self.swarm.behaviour_mut()
                        .kademlia
                        .get_record(kad::RecordKey::new(&umbral_pk_peer_id_key));

                    self.pending.get_umbral_pks.insert(umbral_pk_peer_id, sender.clone());
                };

            }
            NodeCommand::RespondCfrag { agent_name, frag_num, channel } => {

                self.log(format!("RespondCfrags: Finding topic: {}", agent_name));

                let cfrag_indexed = match self.cfrags.get(&agent_name) {
                    None => None,
                    Some(cfrag) => {
                        self.log(format!("RespondCfrags: Found Cfrag: {:?}", cfrag));

                        let cfrag_indexed_bytes = serde_json::to_vec::<Option<CapsuleFragmentIndexed>>(&Some(cfrag.clone()))
                            .expect("serde_json frag fail");

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
                frag_num,
                peer,
                sender,
            } => {
                let request_id = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer, FragmentRequest(agent_name, frag_num));

                self.pending.request_fragments.insert(request_id, sender);
            }
            NodeCommand::RespondFile { file, channel } => {
                self.swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(channel, FragmentResponse(file))
                    .expect("Connection to peer to be still open.");
            }
        }

    }

}
