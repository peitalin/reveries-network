use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use color_eyre::{eyre, Result};

use jsonrpsee::client_transport::ws::{Url, WsTransportClientBuilder};
use jsonrpsee::core::client::{Client, ClientBuilder};


pub fn parse_url(socket_addr: &SocketAddr) -> Result<Url> {
    Url::parse(&format!("ws://{}", socket_addr))
        .map_err(|e| eyre::anyhow!(e.to_string()))
}

pub async fn create_rpc_client(socket_addr: &SocketAddr) -> Result<Client> {
    let url = parse_url(socket_addr)?;
    println!("connecting to {}", url);

    let (
        tx,
        rx
    ) = WsTransportClientBuilder::default().build(url).await?;

    let client: Client = ClientBuilder::default()
        .build_with_tokio(tx, rx);

    Ok(client)
}

