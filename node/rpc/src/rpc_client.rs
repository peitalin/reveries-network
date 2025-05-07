use std::net::SocketAddr;
use color_eyre::{eyre, Result};

use jsonrpsee::client_transport::ws::{Url as WsUrl, WsTransportClientBuilder};
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::core::client::{Client, ClientBuilder as CoreClientBuilder};

#[allow(dead_code)]
pub fn parse_ws_url(socket_addr: &SocketAddr) -> Result<WsUrl> {
    WsUrl::parse(&format!("ws://{}", socket_addr))
        .map_err(|e| eyre::eyre!(e.to_string()))
}

#[allow(dead_code)]
pub async fn create_ws_rpc_client(socket_addr: &SocketAddr) -> Result<Client> {
    let url = parse_ws_url(socket_addr)?;
    println!("Node WebSocket RPC Client connecting on: {}", url);

    let (
        tx,
        rx
    ) = WsTransportClientBuilder::default().build(url).await?;

    let client: Client = CoreClientBuilder::default()
        .build_with_tokio(tx, rx);

    Ok(client)
}

#[allow(dead_code)]
pub async fn create_http_rpc_client(socket_addr: &SocketAddr) -> Result<HttpClient> {
    let url = format!("http://{}", socket_addr);
    println!("Node HTTP RPC Client connecting on: {}", url);

    let client = HttpClientBuilder::default()
        .build(&url)
        .map_err(|e| eyre::anyhow!("Failed to build HTTP client: {}", e))?;

    Ok(client)
}

