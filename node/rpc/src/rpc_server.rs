use std::net::SocketAddr;
use p2p_network::behaviour::{UmbralPeerId, UmbralPublicKeyResponse};
use serde::{Deserialize, Serialize};
use jsonrpsee::types::{ErrorObjectOwned, ErrorObject, ErrorCode};
use jsonrpsee::server::{RpcModule, Server};
use p2p_network::node_client::NodeClient;


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


pub async fn run_server(rpc_port: u16, network_client: NodeClient) -> color_eyre::Result<SocketAddr> {

	let server = Server::builder().build(format!("0.0.0.0:{}", rpc_port)).await?;
	let mut module = RpcModule::new(());

    // register RPC endpoints
	module.register_async_method("broadcast", move |params, _, _| {

        let (
            agent_name,
            shares,
            threshold
        ) = params.parse::<(String, usize, usize)>().expect("error parsing param");

        let nc = network_client.clone();

        async move {
            nc.clone()
                .broadcast_kfrags(agent_name, shares, threshold)
                .await
                .map_err(|e| RpcError(e.to_string()))
        }
    })?;

	let addr = server.local_addr()?;

	let handle = server.start(module);
	// In this example we don't care about doing shutdown so let's it run forever.
	// You may use the `ServerHandle` to shut it down or manage it yourself.
	tokio::spawn(handle.stopped()).await?;

	Ok(addr)
}









