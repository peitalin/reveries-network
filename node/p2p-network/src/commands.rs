use std::{
    collections::{HashMap, HashSet}, error::Error,
};
// use futures::channel::oneshot;
use tokio::sync::oneshot;

use libp2p::{
    request_response::ResponseChannel,
    PeerId,
    Multiaddr
};
use crate::behaviour::{
    CapsuleFragmentIndexed, FragmentResponse, KeyFragmentMessage, UmbralPublicKeyResponse
};


pub enum NodeCommand {
    /// Gets Umbral PKs from connected Peers
    GetPeerUmbralPublicKeys {
        sender: tokio::sync::mpsc::Sender<UmbralPublicKeyResponse>,
    },
    /// Gets Peers that hold the Kfrags for an agent so we can RequestCfrags
    GetKfragPeers {
        agent_name: String,
        sender: oneshot::Sender<HashMap<u32, HashSet<PeerId>>>,
    },
    /// Broadcasts Kfrags to peers (multicasts to specific fragment channels)
    BroadcastKfrags(KeyFragmentMessage),
    ///
    RequestCfrags {
        agent_name: String,
        frag_num: usize,
        sender: oneshot::Sender<Result<Vec<u8>, Box<dyn Error + Send>>>,
    },
    RespondCfrag {
        agent_name: String,
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
        sender: oneshot::Sender<HashSet<PeerId>>,
    },
    RequestFile {
        agent_name: String,
        frag_num: Option<usize>,
        peer: PeerId,
        sender: oneshot::Sender<Result<Vec<u8>, Box<dyn Error + Send>>>,
    },
    RespondFile {
        file: Vec<u8>,
        channel: ResponseChannel<FragmentResponse>,
    }
}