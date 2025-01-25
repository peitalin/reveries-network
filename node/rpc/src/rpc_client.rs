
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use anyhow::Result;

use jsonrpsee::client_transport::ws::{Url, WsTransportClientBuilder};
use jsonrpsee::core::client::{Client, ClientBuilder};

pub async fn create_rpc_client(rpc_port: u16) -> Result<Client> {

    let addr = (IpAddr::V4(Ipv4Addr::LOCALHOST), rpc_port);
    let server_addr = SocketAddr::new(addr.0, addr.1);

    let url = Url::parse(&format!("ws://{}", server_addr))?;

    let (
        tx,
        rx
    ) = WsTransportClientBuilder::default().build(url).await?;

    let client: Client = ClientBuilder::default()
        .build_with_tokio(tx, rx);

    Ok(client)
}

