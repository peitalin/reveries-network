use std::{
    collections::HashSet,
    error::Error,
    path::PathBuf,
    marker::Send,
};
use anyhow::Result;
use futures::{
    SinkExt,
    channel::oneshot,
    prelude::*,
};
use libp2p::PeerId;
use crate::behaviour::{
    FileResponse,
};
use crate::commands::NodeCommand;
use crate::SendError;
use super::NodeClient;


impl NodeClient {

    /// Advertise the local node as the provider of the given file on the DHT.
    pub async fn start_providing_file(&mut self, file_name: String, path: PathBuf) {
        self.file_paths.insert(file_name.clone(), path);

        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(NodeCommand::StartProviding { file_name, sender })
            .await
            .expect("Command receiver not to be dropped.");

        self.log(format!("Providing file: {:?}", self.file_paths));
        receiver.await.expect("Sender not to be dropped.");
    }

    /// Find the providers for the given file on the DHT.
    pub async fn get_file_providers(&mut self, file_name: String) -> HashSet<PeerId> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(NodeCommand::GetProviders {
                file_name,
                sender
            })
            .await
            .expect("Command receiver not to be dropped.");

        receiver.await.expect("Sender not to be dropped.")
    }

    pub async fn handle_get_file(&mut self, file_name: &str) -> Result<Vec<u8>> {

        let name = file_name.to_string();
        self.log(format!("Calling handle_get(file_name: {:?})", file_name));

        // Locate all nodes providing the file.
        let providers = self.get_file_providers(name.clone()).await;

        self.log(format!("Located providers: {:?}", providers));
        if providers.is_empty() {
            self.log(format!("Could not find provider for file {}.", file_name));
            return Ok(vec![])
        }

        // Request the content of the file from each node.
        let requests = providers
            .into_iter()
            .map(|peer_id| {
                let name = name.clone();
                let mut nc = self.clone();
                async move { nc.request_file(peer_id, name).await }.boxed()
            });

        // Await the requests, ignore the remaining once a single one succeeds.
        let file_content = futures::future::select_ok(requests)
            .await
            .map_err(|e| SendError(e.to_string()))?
            .0;

        Ok(file_content)
    }

    /// Request the content of the given file from the given peer.
    pub async fn request_file(
        &mut self,
        peer: PeerId,
        file_name: String
    ) -> Result<Vec<u8>, Box<dyn Error + Send>> {

        let (sender, receiver) = oneshot::channel();

        if self.peer_id == peer {
            self.log(format!("Local node is a file provider: {:?}", self.peer_id));
            self.log(format!("Serving from local"));
            match self.file_paths.get(&file_name) {
                Some(path) => {
                    match std::fs::read(&path) {
                        Ok(file) => Ok(file),
                        Err(e) => Err(Box::new(SendError(e.to_string())))
                    }
                }
                None => panic!("file_name not found {}", file_name)
            }
        } else {
            self.log(format!("Requesting file from peer: {}", peer));
            self.command_sender
                .send(NodeCommand::RequestFile {
                    file_name,
                    frag_num: None, // only for PRE kfrags
                    peer,
                    sender
                })
                .await
                .expect("Command receiver not to be dropped.");

            receiver.await
                .expect("Sender not be dropped.")
        }

    }

    pub async fn respond_file(
        &mut self,
        file: Vec<u8>,
        channel: libp2p::request_response::ResponseChannel<FileResponse>,
    ) {
        self.command_sender
            .send(NodeCommand::RespondFile { file, channel })
            .await
            .expect("Command receiver not to be dropped.");
    }
}