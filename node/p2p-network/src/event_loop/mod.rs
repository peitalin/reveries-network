mod event_handlers;
mod gossipsub_handlers;
pub(crate) mod heartbeat;
mod peer_manager;

use std::{
    collections::{HashMap, HashSet},
    error::Error,
};
use anyhow::Result;
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
use runtime::tee_attestation;
use crate::commands::NodeCommand;
use crate::behaviour::{
    Behaviour,
    CapsuleFragmentIndexed,
    ChatMessage,
    ChatTopic,
    FileEvent,
    FileRequest,
    FileResponse,
    KfragsMessage,
    KfragsTopic,
    UmbralPeerId,
    UmbralPublicKeyResponse
};
use peer_manager::{PeerManager};


pub struct EventLoop {
    peer_id: PeerId,
    swarm: Swarm<Behaviour>,
    command_receiver: mpsc::Receiver<NodeCommand>,
    network_event_sender: mpsc::Sender<FileEvent>,
    // chat
    node_name: String,
    chat_receiver: mpsc::Receiver<ChatMessage>,
    topics: HashMap<String, IdentTopic>,
    // Umbral fragments
    cfrags: HashMap<String, CapsuleFragmentIndexed>,
    // kfrags_peers: HashMap<String, HashMap<u32, HashSet<PeerId>>>,
    kfrags_receiver: mpsc::Receiver<KfragsMessage>,
    umbral_key: UmbralKey,
    pending_get_umbral_pks: HashMap<
        UmbralPeerId,
        tokio::sync::mpsc::Sender<UmbralPublicKeyResponse>
    >,

    peer_manager: PeerManager,

    pending: PendingRequests,
    // file sharing
    pending_start_providing: HashMap<kad::QueryId, oneshot::Sender<()>>,
    pending_get_providers: HashMap<kad::QueryId, oneshot::Sender<HashSet<PeerId>>>,
    pending_request_file: HashMap<
        request_response::OutboundRequestId,
        oneshot::Sender<Result<Vec<u8>, Box<dyn Error + Send>>>
    >,
}

struct PendingRequests {
    start_providing: HashMap<kad::QueryId, oneshot::Sender<()>>,
    get_providers: HashMap<kad::QueryId, oneshot::Sender<HashSet<PeerId>>>,
    request_file: HashMap<
        request_response::OutboundRequestId,
        oneshot::Sender<Result<Vec<u8>, Box<dyn Error + Send>>>
    >,
}

impl PendingRequests {
    fn new() -> Self {
        Self {
            start_providing: Default::default(),
            get_providers: Default::default(),
            request_file: Default::default(),
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
        kfrags_receiver: mpsc::Receiver<KfragsMessage>,
        umbral_key: UmbralKey,
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
            pending_get_umbral_pks: HashMap::new(),
            peer_manager: PeerManager::new(),
            pending: PendingRequests::new(),
            pending_start_providing: Default::default(),
            pending_get_providers: Default::default(),
            pending_request_file: Default::default(),
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
                event = self.swarm.select_next_some() => self.handle_event(event).await,
                command = self.command_receiver.next() => match command {
                    Some(c) => self.handle_command(c).await,
                    // Command channel closed, thus shutting down the network event loop.
                    None => return,
                },
                chat_message = self.chat_receiver.next() => match chat_message {
                    Some(cm) => match cm.topic {
                        ChatTopic::Chat => self.broadcast_chat_message(cm).await,
                        _ => {}
                    },
                    None => return
                },
                kfrags_message = self.kfrags_receiver.next() => match kfrags_message {
                    Some(cm) => match cm.topic {
                        KfragsTopic::Kfrag(..) => self.broadcast_kfrag(cm).await,
                        KfragsTopic::RequestCfrags(_) => {
                            self.request_kfrags(cm).await.expect("request kfrags error");
                        },
                        _ => {}
                    }
                    None => return
                },
            }
        }
    }


    async fn broadcast_chat_message(&mut self, message: ChatMessage) {

        self.log(format!("Broadcasting message: {:?}", message));

        /////////// form heartbeat payload here. Move it later
        self.swarm.behaviour_mut().heartbeat.increment_block_height();

        let (
            _tee_quote ,
            tee_quote_bytes
        ) = tee_attestation::generate_tee_attestation()
            .expect("tee attestation generation err");

        // self.swarm.behaviour_mut().heartbeat.set_tee_attestation(
        //     format!("0x{}", hex::encode(tee_attestation_quote.signature.quote_signature))
        // );
        self.swarm.behaviour_mut().heartbeat.set_tee_attestation(tee_quote_bytes);
        //////////

        match self.topics.get(&message.topic.to_string()) {
            Some(topic) => {

                // self.swarm.behaviour_mut().heartbeat.

                match self.swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic.clone(), message.message) {
                        Ok(_) => {}
                        Err(e) => println!("Publish err: {:?}", e),
                    }
            }
            None => {
                self.log(format!("Err: topic does not exist {:?}", message.topic));
                self.log(format!("Topics subscribed:"));
                let _ = self.swarm.behaviour_mut().gossipsub.topics()
                    .into_iter()
                    .map(|t| println!("{:?}", t)).collect::<()>();
            }
        }
    }

    async fn handle_command(&mut self, command: NodeCommand) {
        match command {
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

                    self.pending_get_umbral_pks.insert(umbral_pk_peer_id, sender.clone());
                };

            }
            NodeCommand::GetRequestKfragPeers { agent_name, sender } => {

                let peers = self.peer_manager
                    .get_umbral_kfrag_providers(&agent_name)
                    .expect("kfrag peers missing")
                    .clone();

                let _ = sender.send(peers);
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
                    //// withhold response, and handle futures Err
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
            NodeCommand::StartProviding { file_name, sender } => {

                let query_id = self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .start_providing(file_name.into_bytes().into())
                    .expect("No store error.");

                self.pending_start_providing.insert(query_id, sender);
            }
            NodeCommand::GetProviders { file_name, sender } => {

                self.log(format!("Command::GetProviders filename: {:?}", file_name));
                let query_id = self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .get_providers(file_name.into_bytes().into());

                self.pending_get_providers.insert(query_id, sender);
            }
            NodeCommand::RequestFile {
                file_name,
                frag_num,
                peer,
                sender,
            } => {
                self.log(format!("Command::RequestFile FileRequest({:?}, {:?})", file_name, frag_num));

                let request_id = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer, FileRequest(file_name, frag_num));

                self.pending_request_file.insert(request_id, sender);
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
