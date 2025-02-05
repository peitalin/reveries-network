use std::net::SocketAddr;
use std::collections::{HashMap, HashSet};
use p2p_network::types::{NextTopic, PrevTopic, TopicSwitch};
use serde::{Deserialize, Serialize};
use jsonrpsee::types::{ErrorObjectOwned, ErrorObject, ErrorCode};
use jsonrpsee::server::{RpcModule, Server};
use p2p_network::node_client::NodeClient;
use std::str::FromStr;
use libp2p::PeerId;


#[derive(Deserialize, Debug, Clone, Serialize)]
pub struct RpcError(String);

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "RpcError: {}", self.0)
    }
}

impl std::error::Error for RpcError {}

impl Into<ErrorObjectOwned> for RpcError {
    fn into(self) -> ErrorObjectOwned {
        ErrorObject::owned(
            ErrorCode::code(&ErrorCode::InternalError),
            self.to_string(),
            Some("")
        )
    }
}


pub async fn run_server(rpc_port: usize, network_client: NodeClient) -> color_eyre::Result<SocketAddr> {

	let server = Server::builder().build(format!("0.0.0.0:{}", rpc_port)).await?;
	let mut module = RpcModule::new(());

    ////// register RPC endpoints //////

    // Broadcast
    let nc1 = network_client.clone();
	module.register_async_method("broadcast", move |params, _, _| {

        let (
            agent_name,
            agent_nonce,
            shares,
            threshold
        ) = params.parse::<(String, usize, usize, usize)>().expect("error parsing param");

        let mut nc1 = nc1.clone();
        async move {
            nc1
                .broadcast_kfrags(agent_name, agent_nonce, shares, threshold)
                .await
                .map_err(|e| RpcError(e.to_string()))
        }
    })?;

    // Request Respawn
    let nc2 = network_client.clone();
	module.register_async_method("request", move |params, _, _| {

        let (agent_name, agent_nonce) = params.parse::<(String, usize)>().expect("error parsing param");
        let mut nc2 = nc2.clone();
        async move {
            nc2
                .request_respawn(agent_name, agent_nonce, None)
                .await
                .map_err(|e| RpcError(e.to_string()))
        }
    })?;

    // Get kfrag peers
    let nc3 = network_client.clone();
	module.register_async_method("get_kfrag_peers", move |params, _, _| {

        let (agent_name, agent_nonce) = params.parse::<(String, usize)>().expect("error parsing param");
        let mut nc3 = nc3.clone();
        async move {
            let peers: HashMap<usize, HashSet<PeerId>> = nc3
                .get_agent_kfrag_peers(agent_name, agent_nonce).await;

            Ok::<HashMap<usize, HashSet<PeerId>>, RpcError>(peers)
        }
    })?;

    // Topic Switch
    let nc4 = network_client.clone();
	module.register_async_method("topic_switch", move |params, _, _| {

        let (
            next_agent_name,
            next_agent_nonce,
            total_frags,
            threshold,
            prev_agent_name,
            prev_agent_nonce,
            peer_id,
        ) = params.parse::<(
            String, usize, usize, usize,
            String, usize, String
        )>().expect("error parsing param");

        let mut nc4 = nc4.clone();
        async move {
            let result = nc4
                .broadcast_switch_topic_nc(TopicSwitch {
                    next_topic: NextTopic {
                        agent_name: next_agent_name,
                        agent_nonce: next_agent_nonce,
                        total_frags: total_frags,
                        threshold: threshold,
                    },
                    prev_topic: Some(PrevTopic {
                        agent_name: prev_agent_name,
                        agent_nonce: prev_agent_nonce,
                        peer_id: PeerId::from_str(&peer_id).ok(),
                    })
                }).await.map_err(|e| RpcError(e.to_string()))?;

            Ok::<usize, RpcError>(result)
        }
    })?;

	let addr = server.local_addr()?;
	let handle = server.start(module);
	// In this example we don't care about doing shutdown so let's it run forever.
	// You may use the `ServerHandle` to shut it down or manage it yourself.
	tokio::spawn(handle.stopped()).await?;
	Ok(addr)
}









