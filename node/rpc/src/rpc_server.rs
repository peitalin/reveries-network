use std::net::SocketAddr;
use color_eyre::eyre::{Error, anyhow};
use futures::{Stream, StreamExt, pin_mut};
use hex;
use jsonrpsee::PendingSubscriptionSink;
use jsonrpsee::types::{ErrorObjectOwned, ErrorObject, ErrorCode};
use jsonrpsee::server::{RpcModule, Server, SubscriptionMessage, TrySendError};

use serde::{Deserialize, Serialize};
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;
use alloy_primitives::Address;

use p2p_network::types::{
    NodeKeysWithVesselStatus,
    AccessCondition,
    ReverieId,
    ReverieType,
    Reverie,
    AccessKey,
    ExecuteWithMemoryReverieResult,
    AnthropicQuery,
};
use p2p_network::node_client::{NodeClient, RestartReason};
use p2p_network::get_node_name;
use runtime::llm::AgentSecretsJson;
use runtime::QuoteBody;
use llm_proxy::usage::SignedUsageReport;


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
    network_client: NodeClient<'a>,
    // proxy_verifying_key: Arc<AccessKey>
) -> color_eyre::Result<SocketAddr> {

	let server = Server::builder()
        .set_message_buffer_capacity(20)
        .build(format!("0.0.0.0:{}", rpc_port))
        .await?;

	let addr = server.local_addr()?;
    println!("RPC server running on {}", addr);
	let mut rpc_module = RpcModule::new(());

    ////////////////////////////////////////////////////
    // RPC Endpoints
    ////////////////////////////////////////////////////

    let nc = network_client.clone();
    rpc_module.register_async_method("spawn_agent", move |params, _, _| {

        let (
            agent_secrets_json,
            threshold,
            total_frags,
        ) = params.parse::<(AgentSecretsJson, usize, usize)>().expect("error parsing params");

        let mut nc = nc.clone();
        async move {
            let result = nc
                .spawn_agent(
                    agent_secrets_json,
                    threshold,
                    total_frags,
                ).await.map_err(|e| RpcError(e.to_string()))?;

            Ok::<NodeKeysWithVesselStatus, RpcError>(result)
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

    let nc = network_client.clone();
    rpc_module.register_async_method("spawn_memory_reverie", move |params, _, _| {

        let (
            memory_secrets_json,
            threshold,
            total_frags,
            access_public_key, // user address allowed to access the memory
        ) = params.parse::<(serde_json::Value, usize, usize, String)>().expect("error parsing params");

        // Parse the string directly into an Address
        let access_public_key: Address = access_public_key.parse()
            .expect("error parsing access_public_key string into Address");

        let mut nc = nc.clone();
        async move {
            let result = nc
                .spawn_memory_reverie(
                    memory_secrets_json,
                    threshold,
                    total_frags,
                    AccessCondition::Ecdsa(access_public_key)
                ).await.map_err(|e| RpcError(e.to_string()))?;

            Ok::<Reverie, RpcError>(result)
        }
    })?;

    let nc = network_client.clone();
    rpc_module.register_async_method("delegate_api_key", move |params, _, _| {

        let (
            reverie_id,
            reverie_type,
            access_key,
        ) = params.parse::<(ReverieId, ReverieType, AccessKey)>().expect("error parsing params");

        let mut nc = nc.clone();
        async move {
            nc.delegate_api_key(
                reverie_id,
                reverie_type,
                access_key,
            ).await.map_err(|e| RpcError(e.to_string()))?;

            Ok::<(), RpcError>(())
        }
    })?;

    let nc = network_client.clone();
    rpc_module.register_async_method(
        "execute_with_memory_reverie",
        move |params, _, _| {

            let (
                reverie_id,
                reverie_type,
                access_key,
                anthropic_query,
            ) = params.parse::<(
                ReverieId,
                ReverieType,
                AccessKey,
                AnthropicQuery
            )>().expect("error parsing params");

            let mut nc = nc.clone();
            async move {
                let result = nc.execute_with_memory_reverie(
                    reverie_id,
                    reverie_type,
                    access_key,
                    anthropic_query,
                ).await.map_err(|e| RpcError(e.to_string()))?;

                Ok::<ExecuteWithMemoryReverieResult, RpcError>(result)
            }
        }
    )?;


    let nc = network_client.clone();
    rpc_module.register_async_method("report_usage", move |params, _, _| {

        let signed_usage_report = params.parse::<SignedUsageReport>().expect("error parsing params");

        let mut nc = nc.clone();
        async move {
            nc.report_usage(
                signed_usage_report,
            ).await
            .map_err(|e| RpcError(e.to_string()))?;

            Ok::<(), RpcError>(())
        }
    })?;

    ////////////////////////////////////////////////////
    // Subscriptions
    ////////////////////////////////////////////////////

    rpc_module.register_subscription(
        "subscribe_letter_stream",
        "params_letter_stream",
        "unsubscribe_letter_stream",
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
        "subscribe_hb",
        "notify_hb",
        "unsubscribe_hb",
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
