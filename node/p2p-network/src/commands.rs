use futures::channel::oneshot;
use std::{
    collections::{HashMap, HashSet}, error::Error,
};
use libp2p::{
    request_response::ResponseChannel,
    PeerId,
    Multiaddr
};
use crate::behaviour::{
    UmbralPublicKeyResponse,
    FileResponse,
};


pub enum NodeCommand {
    GetPeerUmbralPublicKeys {
        sender: tokio::sync::mpsc::Sender<UmbralPublicKeyResponse>,
    },
    GetRequestKfragPeers {
        agent_name: String,
        sender: oneshot::Sender<HashMap<u32, HashSet<PeerId>>>,
    },
    RespondCfrags {
        agent_name: String,
        frag_num: u32,
        channel: ResponseChannel<FileResponse>,
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
        frag_num: Option<u32>,
        peer: PeerId,
        sender: oneshot::Sender<Result<Vec<u8>, Box<dyn Error + Send>>>,
    },
    RespondFile {
        file: Vec<u8>,
        channel: ResponseChannel<FileResponse>,
    }
}