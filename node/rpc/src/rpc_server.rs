use std::net::SocketAddr;
use std::collections::{HashMap, HashSet};
use libp2p::PeerId;
use jsonrpsee::types::{ErrorObjectOwned, ErrorObject, ErrorCode};
use jsonrpsee::server::{RpcModule, Server};
use serde::{Deserialize, Serialize};

use p2p_network::types::{
    AgentNameWithNonce,
    NextTopic,
    PrevTopic,
    TopicSwitch,
    UmbralPublicKeyResponse
};
use p2p_network::node_client::{NodeClient, RestartReason};
use runtime::llm::AgentSecretsJson;


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
            Some("RpcError")
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
        ) = params.parse::<(String, usize, usize, usize)>().expect("error parsing params");
        let agent_name_nonce = AgentNameWithNonce(agent_name, agent_nonce);

        let mut nc1 = nc1.clone();
        async move {
            nc1
                .broadcast_kfrags(agent_name_nonce, shares, threshold)
                .await
                .map_err(|e| RpcError(e.to_string()))
        }
    })?;

    let nc2 = network_client.clone();
	module.register_async_method("get_kfrag_broadcast_peers", move |params, _, _| {

        let (agent_name, agent_nonce) = params.parse::<(String, usize)>()
            .expect("error parsing params");
        let agent_name_nonce = AgentNameWithNonce(agent_name, agent_nonce);

        let mut nc = nc2.clone();
        async move {
            let peers: HashMap<usize, HashSet<PeerId>> = nc
                .get_kfrag_broadcast_peers(agent_name_nonce).await;

            Ok::<HashMap<usize, HashSet<PeerId>>, RpcError>(peers)
        }
    })?;

    let nc3 = network_client.clone();
	module.register_async_method("spawn_agent", move |params, _, _| {

        let (
            agent_secrets_json,
            total_frags,
            threshold,
        ) = params.parse::<(AgentSecretsJson, usize, usize)>().expect("error parsing params");

        let mut nc = nc3.clone();
        async move {
            let result = nc
                .spawn_agent(
                    agent_secrets_json,
                    total_frags,
                    threshold,
                ).await.map_err(|e| RpcError(e.to_string()))?;

            Ok::<UmbralPublicKeyResponse, RpcError>(result)
        }
    })?;

    let nc4 = network_client.clone();
	module.register_async_method("trigger_node_failure", move |_params, _, _| {

        let mut nc4 = nc4.clone();
        async move {
            let result = nc4
                .simulate_node_failure().await
                .map_err(|e| RpcError(e.to_string()))?;

            Ok::<RestartReason, RpcError>(result)
        }
    })?;

    let nc5 = network_client.clone();
	module.register_async_method("get_node_state", move |_params, _, _| {

        let mut nc = nc5.clone();
        async move {
            let result = nc
                .get_node_state()
                .await
                .map_err(|e| RpcError(e.to_string()))?;

            Ok::<serde_json::Value, RpcError>(result)
        }
    })?;

	let addr = server.local_addr()?;
	let handle = server.start(module);
	// In this example we don't care about doing shutdown so let's run it forever.
	// You may use the `ServerHandle` to shut it down
	tokio::spawn(handle.stopped()).await?;
	Ok(addr)
}









