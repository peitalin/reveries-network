use std::net::SocketAddr;
use std::collections::{HashMap, HashSet};

use color_eyre::eyre::{Error, anyhow};
use futures::{Stream, StreamExt, pin_mut};

use hex;
use jsonrpsee::PendingSubscriptionSink;
use jsonrpsee::types::{ErrorObjectOwned, ErrorObject, ErrorCode};
use jsonrpsee::server::{RpcModule, Server, SubscriptionMessage, TrySendError};

use serde::{Deserialize, Serialize};
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;

use p2p_network::types::{
    AgentNameWithNonce,
    NodeVesselStatus
};
use p2p_network::node_client::{NodeClient, RestartReason};
use p2p_network::get_node_name;
use runtime::llm::AgentSecretsJson;
use runtime::QuoteBody;


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

	let server = Server::builder()
        .set_message_buffer_capacity(10)
        .build(format!("0.0.0.0:{}", rpc_port))
        .await?;

	let addr = server.local_addr()?;
	let mut rpc_module = RpcModule::new(());

    ////////////////////////////////////////////////////
    // RPC Endpoints
    ////////////////////////////////////////////////////

    let nc = network_client.clone();
	rpc_module.register_async_method("get_kfrag_broadcast_peers", move |params, _, _| {

        let (agent_name, agent_nonce) = params.parse::<(String, usize)>()
            .expect("error parsing params");
        let agent_name_nonce = AgentNameWithNonce(agent_name, agent_nonce);

        let mut nc = nc.clone();
        async move {
            let peers: HashMap<usize, HashSet<libp2p::PeerId>> = nc
                .get_kfrag_broadcast_peers(agent_name_nonce).await;

            Ok::<HashMap<usize, HashSet<libp2p::PeerId>>, RpcError>(peers)
        }
    })?;

    let nc = network_client.clone();
    rpc_module.register_async_method("spawn_agent", move |params, _, _| {

        let (
            agent_secrets_json,
            total_frags,
            threshold,
        ) = params.parse::<(AgentSecretsJson, usize, usize)>().expect("error parsing params");

        let mut nc = nc.clone();
        async move {
            let result = nc
                .spawn_agent(
                    agent_secrets_json,
                    total_frags,
                    threshold,
                ).await.map_err(|e| RpcError(e.to_string()))?;

            Ok::<NodeVesselStatus, RpcError>(result)
        }
    })?;

    let nc = network_client.clone();
	rpc_module.register_async_method("trigger_node_failure", move |_params, _, _| {
        let mut nc = nc.clone();
        async move {
            let result = nc
                .simulate_node_failure().await
                .map_err(|e| RpcError(e.to_string()))?;

            Ok::<RestartReason, RpcError>(result)
        }
    })?;

    let nc = network_client.clone();
	rpc_module.register_async_method("get_node_state", move |_params, _, _| {
        let nc = nc.clone();
        async move {
            let result = nc
                .get_node_state()
                .await
                .map_err(|e| RpcError(e.to_string()))?;

            Ok::<serde_json::Value, RpcError>(result)
        }
    })?;

    ////////////////////////////////////////////////////
    // Subscriptions
    ////////////////////////////////////////////////////

    rpc_module.register_subscription(
        "subscribe_letter_stream",
        "params_letter_stream",
        "unsubscribe_letter_stream",
        // "subscribe_node_state", // subscribe_method_name
        // "notify_node_state",    // notif_method_name
        // "unsubscribe_node_state", // unsubscribe_method_name
        |params, pending_sink, _, _| async move {

			let n = params.one::<usize>().expect("params err");
            const LETTERS: &str = "abcdefghijklmnopqrstuvxyz";

			let stream = IntervalStream::new(
                interval(std::time::Duration::from_millis(400))
            )
            .take(n)
            .enumerate()
            .map(move |(i, _instant)| &LETTERS[i..i+1]);

			pipe_from_stream_and_drop(pending_sink, stream)
                .await.map_err(Into::into)
        }
    )?;

    let nc = network_client.clone();
    rpc_module.register_subscription(
        "subscribe_hb", // subscribe_method_name
        "notify_hb",    // notif_method_name
        "unsubscribe_hb", // unsubscribe_method_name
        move |params, pending_sink, _, _| {

			let _n = params.one::<usize>().expect("params err");
            let heartbeat_receiver = nc.get_hb_channel();
            let nc2 = nc.clone();

            async move {

                let stream = async_stream::stream! {
                    while let Ok(tee_payload) = heartbeat_receiver.recv().await {

                        let node_state_result = nc2.get_node_state().await.ok();

                        let tee_quote_v4 = match tee_payload.latest_tee_attestation.tee_attestation_bytes {
                            None => None,
                            Some(tee_bytes) => {
                                Some(parse_tee_attestation_bytes(tee_bytes))
                            }
                        };

                        let json_payload = serde_json::json!({
                            "tee_attestation": {
                                "peer_id": tee_payload.peer_id,
                                "peer_name": get_node_name(&tee_payload.peer_id),
                                "tee_quote_v4": tee_quote_v4
                            },
                            "node_state": node_state_result,
                            "time": get_time(),
                            // get duration between heartbeats etc
                        });
                        yield Some(json_payload)
                    }
                };
                pin_mut!(stream);

                pipe_from_stream_and_drop(pending_sink, stream)
                    .await.map_err(Into::into)
            }
        }
    )?;

    ////////////////////////////////////////////////////
	let handle = server.start(rpc_module);
	// In this example we don't care about doing shutdown so let's run it forever.
	// You may use the `ServerHandle` to shut it down
	tokio::spawn(handle.stopped()).await?;
	Ok(addr)
}


pub async fn pipe_from_stream_and_drop<T: Serialize>(
	pending_sink: PendingSubscriptionSink,
	mut stream: impl Stream<Item = T> + Unpin,
) -> Result<(), Error> {

	let mut sink = pending_sink.accept().await?;

	loop {
		tokio::select! {
			maybe_item = stream.next() => {
				let item = match maybe_item {
					Some(item) => item,
					None => break Err(anyhow!("Subscription closed")),
				};
				let msg = SubscriptionMessage::from_json(&item)?;
				match sink.try_send(msg) {
					Ok(_) => {},
					Err(TrySendError::Closed(e)) => {
                        break Err(anyhow!("Subscription closed {:?}", e))
                    },
					// channel is full, let's be naive and just drop the message.
					Err(TrySendError::Full(_)) => {},
				}
			}
		}
	}
}


pub fn get_time() -> std::time::Duration {
    let start = std::time::SystemTime::now();
    let now = start
        .duration_since(std::time::UNIX_EPOCH)
        .expect("Time went backwards");
    now
}

pub fn parse_tee_attestation_bytes(ta_bytes: Vec<u8>) -> serde_json::Value {

    let quote = runtime::tee_attestation::QuoteV4::from_bytes(&ta_bytes);
    let now = get_time();

    let quote_body = match quote.quote_body {
        // no deserializer traits on these types, defined in dcap-rs crate
        QuoteBody::SGXQuoteBody(e) => {
            format!("{:?}", &e)
        }
        QuoteBody::TD10QuoteBody(e) => {
            format!("{:?}", &e)
        }
    };

    serde_json::json!({
        "header": &format!("{:?}", &quote.header),
        "signature": {
            "ecdsa_attestation_key": hex::encode(&quote.signature.ecdsa_attestation_key),
            "qe_cert_data": format!("{:?}", &quote.signature.qe_cert_data),
            "quote_signature": hex::encode(&quote.signature.quote_signature),
        },
        "signature_len": &format!("{}", &quote.signature_len),
        "quote_body": quote_body,
        "time": now,
    })
}
