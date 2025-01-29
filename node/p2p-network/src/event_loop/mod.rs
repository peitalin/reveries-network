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
use crate::{commands::NodeCommand, get_node_name};
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
    cfrags: HashMap<String, CapsuleFragmentIndexed>,

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
                                let topic = KfragsTopic::RequestCfrags(agent_name.to_string());
                            } else {
                                self.log(format!("{} died but wasn't hosting agent.", node_name).purple());
                            }
                        }
                    };

                    if let Some(agent_name) = failed_agents.iter().next() {
                        let topic = KfragsTopic::RequestCfrags(agent_name.to_string());
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
                kfrags_message = self.kfrags_receiver.recv() => match kfrags_message {
                    Some(cm) => match cm.topic {
                        KfragsTopic::BroadcastKfrag(..) => {
                            self.broadcast_kfrag(cm).await;
                        },
                        KfragsTopic::RequestCfrags(agent_name) => {
                            println!(">>>>>>>>> DDDDDOES NOTHING?? :{}", agent_name);
                            self.get_cfrags2(agent_name).await;
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
            NodeCommand::RequestCfrags { agent_name, frag_num, sender } => {

                let all_providers = self.peer_manager
                    .get_umbral_kfrag_providers(&agent_name)
                    .expect("kfrag peers missing")
                    .clone();


                let providers = all_providers.iter().filter_map(|(&kfrag_num, peers)| {
                    if frag_num == kfrag_num as usize {
                        Some(peers)
                    } else {
                        None
                    }
                }).flat_map(|v| v.iter().collect::<Vec<&PeerId>>())
                .collect::<Vec<&PeerId>>();

                self.log(format!("Located Cfrag broadcast peers: {:?}\n", providers));
                if providers.is_empty() {
                    self.log(format!("Could not find provider for agent_name {}", agent_name));
                }

                let peer_id =  match self.cfrags.get(&agent_name) {
                    None => providers.iter().next().unwrap(),
                    Some(cfrag) => {
                        // filter peers that are the vessel node
                        let peer_id = providers.iter()
                            .filter_map(|&peer_id| {
                                if *peer_id != cfrag.vessel_peer_id {
                                    Some(peer_id)
                                } else {
                                    None
                                }
                            })
                            .next().unwrap();

                        peer_id
                    }
                };

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
            NodeCommand::RespondCfrags { agent_name, frag_num, channel } => {

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


    pub async fn get_cfrags2(&mut self, agent_name: String) {

        let providers = self.peer_manager
            .get_umbral_kfrag_providers(&agent_name)
            .expect("kfrag peers missing")
            .clone();

        self.log(format!("Located Cfrag broadcast peers: {:?}\n", providers));
        if providers.is_empty() {
            self.log(format!("Could not find provider for agent_name {}", agent_name));
        }

        let mut capsule_frags: HashMap<u32, VerifiedCapsuleFrag> = HashMap::new();
        let mut new_vessel_pk_vec: Vec<CapsuleFragmentIndexed> = Vec::new();

        // For each key_fragment(n)
        for &kfrag_num in providers.keys() {

            let providers = providers.iter()
                .filter(|(&frag_num, _hset)| frag_num == kfrag_num)
                .map(|v| {
                    v.1
                })
                .next()
                .expect("error filtering providers...");

            println!("AAAAAA");

            let (sender, receiver) = oneshot::channel();

            let peer_id = providers.iter().next().unwrap();

            let request_id = self
                .swarm
                .behaviour_mut()
                .request_response
                .send_request(&peer_id, FragmentRequest(agent_name.clone(), Some(kfrag_num as usize)));

            println!("BBBBBBBBB");

            self.pending.request_fragments.insert(request_id, sender);

            println!("CCCCCC");


            let data = receiver.await.expect("Sender not be dropped.");

            println!("DDDDDDd");

            // Await the requests, ignore the remaining once a single one succeeds.
            if let Ok(cfrag_raw_bytes) = data {
                if let Ok(cfrag_indexed) = serde_json::from_slice::<CapsuleFragmentIndexed>(&cfrag_raw_bytes) {

                    let new_vessel_pk  = cfrag_indexed.bob_pk;
                    self.log(format!("Received Cfrag({}): \n{}\n", kfrag_num, cfrag_indexed));

                    // Bob must check that cfrags are valid
                    // assemble kfrags, verify them as cfrags.
                    let verified_cfrag = cfrag_indexed.cfrag.clone().verify(
                        &cfrag_indexed.capsule.clone().unwrap(),
                        &cfrag_indexed.verifying_pk, // verifying pk
                        &cfrag_indexed.alice_pk, // alice pk
                        &new_vessel_pk // bob pk
                    ).expect("Error verifying Cfrag");

                    self.log(format!("Verified Cfrag({}): \n{}\n", kfrag_num, verified_cfrag));

                    new_vessel_pk_vec.push(cfrag_indexed);
                    capsule_frags.insert(kfrag_num, verified_cfrag);
                }
            } else {
                self.log(format!("Future error in Cfrag({})", kfrag_num));
            }
        }

        let mut new_vessel_pk= new_vessel_pk_vec.pop().unwrap();
        let threshold = new_vessel_pk.threshold as usize;
        let num_frags = capsule_frags.len();
        self.log(format!("Received {}/{} required CapsuleFrags", num_frags, threshold));

        let verified_cfrags = capsule_frags.into_iter()
            .map(|(index, verified_cfrags)| verified_cfrags)
            .collect::<Vec<VerifiedCapsuleFrag>>();

        // Bob opens the capsule by using at least `threshold` cfrags,
        // and then decrypts the re-encrypted ciphertext.
        match umbral_pre::decrypt_reencrypted(
            &self.umbral_key.secret_key, // bob
            &new_vessel_pk.alice_pk, // alice
            &new_vessel_pk.capsule.as_mut().unwrap(),
            verified_cfrags,
            new_vessel_pk.ciphertext.as_mut().unwrap()
        ) {
            Ok(plaintext_bob) => {

                let decrypted_data = serde_json::from_slice::<serde_json::Value>(&plaintext_bob)
                    .expect("error marshalling decrypted plaintext to JSON data");

                let agent_secrets_str = serde_json::to_string_pretty(&decrypted_data)
                    .expect("to_string_pretty error");

                self.log(format!("Decrypted (re-encrypted) agent data:\n{}", agent_secrets_str));

                let agent_secrets_json = serde_json::from_slice::<AgentSecretsJson>(&plaintext_bob)
                    .expect("parse AgentSecretJson error");

                // if let Some(anthropic_api_key) = agent_secrets_json.anthropic_api_key {
                //     self.log(format!("Decrypted Anthropic key, querying Claude:"));
                //     let _ = test_claude_query(anthropic_api_key).await;
                // }

            },
            Err(e) => {
                let node_name = get_node_name(&self.peer_id);
                self.log(format!(">>> Err({})", e));
                if (num_frags < threshold) {
                    self.log(format!(">>> Not enough fragments. Need {threshold}, received {num_frags}"));
                } else {
                    self.log(format!(">>> Not decryptable by user {} with: {}", node_name, self.umbral_key.public_key));
                    self.log(format!(">>> Only decryptable by new vessel with: {}", new_vessel_pk.bob_pk));
                }
            }
        };
    }
}
