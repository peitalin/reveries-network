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
    CapsuleFragmentIndexed, FragmentResponse, UmbralPublicKeyResponse
};


pub enum NodeCommand {
    GetPeerUmbralPublicKeys {
        sender: tokio::sync::mpsc::Sender<UmbralPublicKeyResponse>,
    },
    GetRequestKfragPeers {
        agent_name: String,
        sender: oneshot::Sender<HashMap<u32, HashSet<PeerId>>>,
    },
    RequestCfrags {
        agent_name: String,
        frag_num: usize,
        // sender: tokio::sync::mpsc::Sender<Result<Vec<u8>, Box<dyn Error + Send>>>,
        sender: oneshot::Sender<Result<Vec<u8>, Box<dyn Error + Send>>>,
    },
    RespondCfrags {
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