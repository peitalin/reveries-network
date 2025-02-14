
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use color_eyre::{eyre, Result};

use jsonrpsee::client_transport::ws::{Url, WsTransportClientBuilder};
use jsonrpsee::core::client::{Client, ClientBuilder};


pub fn parse_url(rpc_port: u16) -> Result<Url> {
    let addr = (IpAddr::V4(Ipv4Addr::LOCALHOST), rpc_port);
    let server_addr = SocketAddr::new(addr.0, addr.1);
    Url::parse(&format!("ws://{}", server_addr))
        .map_err(|e| eyre::anyhow!(e.to_string()))
}

pub async fn create_rpc_client(rpc_port: u16) -> Result<Client> {

    let url = parse_url(rpc_port)?;

    let (
        tx,
        rx
    ) = WsTransportClientBuilder::default().build(url).await?;

    let client: Client = ClientBuilder::default()
        .build_with_tokio(tx, rx);

    Ok(client)
}

