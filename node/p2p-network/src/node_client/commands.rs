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
    AgentNameWithNonce,
    FragmentNumber,
    FragmentResponseEnum,
    KeyFragmentMessage,
    TopicSwitch,
    UmbralPublicKeyResponse
};

use super::container_manager::RestartReason;


pub enum NodeCommand {
    /// Gets Umbral PublicKey(s) of peers that hold Agent fragments from Kademlia
    /// Looks up which peers hold an AgentNameWithNonce's fragments, then retrieves their
    /// PRE key from Kademlia
    GetPeerUmbralPublicKeys {
        agent_name_nonce: AgentNameWithNonce,
        sender: mpsc::Sender<UmbralPublicKeyResponse>,
    },
    /// Gets Peers that are subscribed to the Kfrag Broadcast channel for an agent
    /// Note: Just because peers are subscribed to the same broadcast,
    /// doesnt meant that peer has the fragments yet. Need to use GetKfragProviders.
    GetKfragBroadcastPeers {
        agent_name_nonce: AgentNameWithNonce,
        // returns: {frag_num: [peer_id]}
        sender: oneshot::Sender<HashMap<usize, HashSet<PeerId>>>,
    },
    /// Gets Peers that hold the Kfrags for an agent.
    /// KfragProviders = kfrag holders
    /// KfragBroadcastPeers = peers subscribed to a Kfrag broadcast channel
    GetKfragProviders {
        agent_name_nonce: AgentNameWithNonce,
        sender: oneshot::Sender<HashSet<PeerId>>,
    },

    /// Broadcasts Kfrags to peers (multicasts to specific fragment channels)
    BroadcastKfrags(KeyFragmentMessage),

    RespondCfrag {
        agent_name_nonce: AgentNameWithNonce,
        frag_num: usize,
        sender_peer_id: PeerId, // peer who sends cfrag back
        channel: ResponseChannel<FragmentResponseEnum>,
    },
    SwitchTopic(
        TopicSwitch,
        oneshot::Sender<usize>,
    ),
    SaveKfragProvider {
        agent_name_nonce: AgentNameWithNonce,
        frag_num: usize,
        sender_peer_id: PeerId, // peer who holds the kfrag
        channel: ResponseChannel<FragmentResponseEnum>,
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
    /// Request Capsule Fragments for threshold decryption
    RequestFragment {
        agent_name_nonce: AgentNameWithNonce,
        frag_num: FragmentNumber,
        peer: PeerId, // peer to request fragment from
        sender: oneshot::Sender<Result<Vec<u8>, SendError>>,
    },
    TriggerRestart {
        sender: oneshot::Sender<RestartReason>,
        reason: RestartReason,
    }
}