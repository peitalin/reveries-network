use std::collections::{HashMap, HashSet};
use std::error::Error;
use libp2p::{
    request_response::ResponseChannel,
    PeerId,
    Multiaddr
};
use tokio::sync::{mpsc, oneshot};
use crate::SendError;
use crate::types::{
    KeyFragmentMessage,
    FragmentResponse,
    TopicSwitch,
    UmbralPublicKeyResponse
};


pub enum NodeCommand {
    /// Gets Umbral PKs from connected Peers
    GetPeerUmbralPublicKey {
        agent_name: String,
        agent_nonce: usize,
        sender: mpsc::Sender<UmbralPublicKeyResponse>,
    },
    /// Gets Peers that are subscribed to the Kfrag Broadcast channel for an agent

    /// Gets Peers that have the Kfrags for an agent
    GetKfragPeers {
        agent_name: String,
        agent_nonce: usize,
        // returns: {frag_num: [peer_id]}
        sender: oneshot::Sender<HashMap<usize, HashSet<PeerId>>>,
    },
    /// Broadcasts Kfrags to peers (multicasts to specific fragment channels)
    BroadcastKfrags(KeyFragmentMessage),

    RespondCfrag {
        agent_name: String,
        agent_nonce: usize,
        frag_num: usize,
        sender_peer: PeerId, // peer who sends cfrag back
        channel: ResponseChannel<FragmentResponse>,
    },
    SwitchTopic(
        TopicSwitch,
        oneshot::Sender<usize>,
    ),
    StartListening {
        addr: Multiaddr,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    GetProviders {
        agent_name: String,
        agent_nonce: usize,
        sender: oneshot::Sender<HashSet<PeerId>>,
    },
    SubscribeTopics {
        topics: Vec<String>,
        sender: oneshot::Sender<Vec<String>>,
    },
    UnsubscribeTopics {
        topics: Vec<String>,
        sender: oneshot::Sender<Vec<String>>,
    },
    /// Request Capsule Fragments for threshold decryption
    RequestFragment {
        agent_name: String,
        agent_nonce: usize,
        frag_num: Option<usize>,
        peer: PeerId, // peer to request fragment from
        sender: oneshot::Sender<Result<Vec<u8>, SendError>>,
    },
}