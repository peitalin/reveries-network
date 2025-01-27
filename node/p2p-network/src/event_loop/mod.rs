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
    channel::{mpsc, oneshot},
    StreamExt,
};
use libp2p::{
    gossipsub::IdentTopic,
    kad,
    request_response,
    swarm::Swarm,
    PeerId
};
use runtime::reencrypt::UmbralKey;
use crate::commands::NodeCommand;
use crate::behaviour::{
    Behaviour,
    CapsuleFragmentIndexed,
    ChatMessage,
    FileEvent,
    FragmentRequest,
    FileResponse,
    KfragsBroadcastMessage,
    KfragsTopic,
    UmbralPeerId,
    UmbralPublicKeyResponse
};
use peer_manager::PeerManager;


pub struct EventLoop {
    swarm: Swarm<Behaviour>,
    command_receiver: mpsc::Receiver<NodeCommand>,
    network_event_sender: mpsc::Sender<FileEvent>,

    // chat
    peer_id: PeerId,
    node_name: String,
    chat_receiver: mpsc::Receiver<ChatMessage>,
    topics: HashMap<String, IdentTopic>,

    // Umbral fragments
    umbral_key: UmbralKey,
    cfrags: HashMap<String, CapsuleFragmentIndexed>,
    kfrags_receiver: mpsc::Receiver<KfragsBroadcastMessage>,

    // tracks peer heartbeats status
    peer_manager: PeerManager,

    // pending P2p network requests
    pending: PendingRequests,

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
        kfrags_receiver: mpsc::Receiver<KfragsBroadcastMessage>,
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
            heartbeat_failure_receiver,
        }
    }

    fn log(&self, message: String) {
        println!("{} {}{} {}",
            "NetworkEvent".green(), self.node_name.yellow(), ">".blue(),
            message
        );
    }

    pub async fn listen_for_commands_and_events(mut self) {
        loop {
            tokio::select! {
                swarm_event = self.swarm.select_next_some() => self.handle_swarm_event(swarm_event).await,
                heartbeat = self.heartbeat_failure_receiver.recv() => match heartbeat {
                    Some(hb) => self.handle_heartbeat_failure(hb).await,
                    None => break // channel closed, shutting down the network event loop.
                },
                command = self.command_receiver.next() => match command {
                    Some(c) => self.handle_command(c).await,
                    None => return
                },
                chat_message = self.chat_receiver.next() => match chat_message {
                    Some(cm) => self.broadcast_chat_message(cm).await,
                    None => return
                },
                kfrags_message = self.kfrags_receiver.next() => match kfrags_message {
                    Some(cm) => match cm.topic {
                        KfragsTopic::Kfrag(..) => {
                            self.broadcast_kfrag(cm).await.expect("todo: handle expect");
                        },
                        KfragsTopic::RequestCfrags(..) => {
                            self.request_kfrags(cm).await.expect("todo: handle expect");
                        },
                        KfragsTopic::Unknown(s) => {
                            self.log(format!("Unknown KfragsTopic: {}", s));
                        }
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
            NodeCommand::GetRequestKfragPeers { agent_name, sender } => {
                let peers = self.peer_manager
                    .get_umbral_kfrag_providers(&agent_name)
                    .expect("kfrag peers missing")
                    .clone();

                let _ = sender.send(peers);
            }
            NodeCommand::GetProviders { agent_name, sender } => {

                self.log(format!("Command::GetProviders filename: {:?}", agent_name));
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
            NodeCommand::RespondCfrags { agent_name, frag_num, channel } => {

                self.log(format!("RespondCfrags: Finding topic: {}", agent_name));
                if let Some(cfrag_indexed) = self.cfrags.get(&agent_name) {

                    let cfrag_indexed_bytes = serde_json::to_vec(cfrag_indexed)
                        .expect("serde_json frag fail");

                    self.log(format!("RespondCfrags: Found Cfrag: {:?}", cfrag_indexed));
                    self.swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, FileResponse(cfrag_indexed_bytes))
                        .expect("Connection to peer to be still open.");
                } else {
                    self.log(format!("No Cfrags found..."));
                    //// withhold response, and handle futures Err? or send Err
                    // self.swarm
                    //     .behaviour_mut()
                    //     .request_response
                    //     .send_response(channel, FileResponse(vec![]))
                    //     .expect("Connection to peer to be still open.");
                }
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
                    .send_response(channel, FileResponse(file))
                    .expect("Connection to peer to be still open.");
            }
        }

    }
}
