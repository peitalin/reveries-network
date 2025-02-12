use std::net::SocketAddr;
use std::collections::{HashMap, HashSet};
use p2p_network::types::{AgentNameWithNonce, NextTopic, PrevTopic, TopicSwitch, UmbralPublicKeyResponse};
use runtime::llm::AgentSecretsJson;
use serde::{Deserialize, Serialize};
use jsonrpsee::types::{ErrorObjectOwned, ErrorObject, ErrorCode};
use jsonrpsee::server::{RpcModule, Server};
use p2p_network::node_client::{NodeClient, RestartReason};
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


pub async fn run_server<'a: 'static>(
    rpc_port: usize,
    network_client: NodeClient<'a>
) -> color_eyre::Result<SocketAddr> {

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
        let agent_name_nonce = AgentNameWithNonce(agent_name, agent_nonce);

        let mut nc1 = nc1.clone();
        async move {
            nc1
                .broadcast_kfrags(agent_name_nonce, shares, threshold)
                .await
                .map_err(|e| RpcError(e.to_string()))
        }
    })?;

    // Request Respawn
    let nc2 = network_client.clone();
	module.register_async_method("request", move |params, _, _| {

        let (agent_name, agent_nonce) = params.parse::<(String, usize)>().expect("error parsing param");
        let agent_name_nonce = AgentNameWithNonce(agent_name, agent_nonce);

        let mut nc2 = nc2.clone();
        async move {
            nc2
                .request_respawn(agent_name_nonce, None)
                .await
                .map_err(|e| RpcError(e.to_string()))
        }
    })?;

    // Get kfrag peers
    let nc3 = network_client.clone();
	module.register_async_method("get_kfrag_broadcast_peers", move |params, _, _| {

        let (agent_name, agent_nonce) = params.parse::<(String, usize)>().expect("error parsing param");
        let agent_name_nonce = AgentNameWithNonce(agent_name, agent_nonce);

        let mut nc3 = nc3.clone();
        async move {
            let peers: HashMap<usize, HashSet<PeerId>> = nc3
                .get_kfrag_broadcast_peers(agent_name_nonce).await;

            Ok::<HashMap<usize, HashSet<PeerId>>, RpcError>(peers)
        }
    })?;

    // Spawn Agent
    let nc4 = network_client.clone();
	module.register_async_method("spawn_agent", move |params, _, _| {

        let (
            agent_secrets_json,
            total_frags,
            threshold,
        ) = params.parse::<(AgentSecretsJson, usize, usize)>().expect("error parsing param");

        let mut nc4 = nc4.clone();
        async move {
            let result = nc4
                .spawn_agent(
                    agent_secrets_json,
                    total_frags,
                    threshold,
                ).await.map_err(|e| RpcError(e.to_string()))?;

            Ok::<UmbralPublicKeyResponse, RpcError>(result)
        }
    })?;

    // Topic Switch
    let nc4 = network_client.clone();
	module.register_async_method("trigger_node_failure", move |params, _, _| {

        let mut nc4 = nc4.clone();
        async move {
            let result = nc4
                .simulate_node_failure().await
                .map_err(|e| RpcError(e.to_string()))?;

            Ok::<RestartReason, RpcError>(result)
        }
    })?;

	let addr = server.local_addr()?;
	let handle = server.start(module);
	// In this example we don't care about doing shutdown so let's it run forever.
	// You may use the `ServerHandle` to shut it down or manage it yourself.
	tokio::spawn(handle.stopped()).await?;
	Ok(addr)
}









