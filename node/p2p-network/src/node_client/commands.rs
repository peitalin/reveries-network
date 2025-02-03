use std::collections::{HashMap, HashSet};
use std::error::Error;
use libp2p::{
    request_response::ResponseChannel,
    PeerId,
    Multiaddr
};
use tokio::sync::{mpsc, oneshot};
use crate::SendError;
use crate::behaviour::KeyFragmentMessage;
use crate::types::{
    FragmentResponse,
    UmbralPublicKeyResponse
};


pub enum NodeCommand {
    /// Gets Umbral PKs from connected Peers
    GetPeerUmbralPublicKey {
        agent_name: String,
        agent_nonce: usize,
        sender: mpsc::Sender<UmbralPublicKeyResponse>,
    },
    /// Gets Peers that are subscribed to the Kfrag Broadcast for an agent
    GetKfragBroadcastPeers {
        agent_name: String,
        agent_nonce: usize,
        sender: oneshot::Sender<HashMap<u32, HashSet<PeerId>>>,
    },
    /// Broadcasts Kfrags to peers (multicasts to specific fragment channels)
    BroadcastKfrags(KeyFragmentMessage),
    /// Request Capsule Fragments for threshold decryption
    RequestCfrags {
        agent_name: String,
        agent_nonce: usize,
        frag_num: usize,
        // sender: oneshot::Sender<Result<Vec<u8>, Box<dyn Error + Send>>>,
        sender: oneshot::Sender<Result<Vec<u8>, SendError>>,
    },
    RespondCfrag {
        agent_name: String,
        agent_nonce: usize,
        frag_num: usize,
        channel: ResponseChannel<FragmentResponse>,
    },
    StartListening {
        addr: Multiaddr,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    SubscribeTopics {
        topics: Vec<String>,
        sender: oneshot::Sender<Vec<String>>,
    },
    UnsubscribeTopics {
        topics: Vec<String>,
        sender: oneshot::Sender<Vec<String>>,
    },
    GetProviders {
        agent_name: String,
        agent_nonce: usize,
        sender: oneshot::Sender<HashSet<PeerId>>,
    },
    RequestFile {
        agent_name: String,
        agent_nonce: usize,
        frag_num: Option<usize>,
        peer: PeerId,
        // sender: oneshot::Sender<Result<Vec<u8>, Box<dyn Error + Send>>>,
        sender: oneshot::Sender<Result<Vec<u8>, SendError>>,
    },
    RespondFile {
        file: Vec<u8>,
        channel: ResponseChannel<FragmentResponse>,
    }
}