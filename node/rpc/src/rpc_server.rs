use std::net::SocketAddr;
use color_eyre::eyre::{Result, Error, ErrReport};
use futures::{Stream, StreamExt, pin_mut};
use hex;
use jsonrpsee::PendingSubscriptionSink;
use jsonrpsee::types::{ErrorObjectOwned, ErrorObject, ErrorCode, Params};
use jsonrpsee::server::{
    RpcModule,
    Server,
    SubscriptionMessage,
    TrySendError,
    MethodCallback,
    RegisterMethodError,
    Extensions,
    IntoResponse,
};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;
use alloy_primitives::Address;

use p2p_network::types::{
    AccessCondition,
    ReverieId,
    ReverieType,
    AccessKey,
    AnthropicQuery,
};
use p2p_network::node_client::NodeClient;
use p2p_network::get_node_name;
use runtime::llm::AgentSecretsJson;
use runtime::QuoteBody;
use llm_proxy::usage::SignedUsageReport;

pub struct RpcServer {
    pub server: Server,
    pub node_client: NodeClient, // for mutable node_client
    pub rpc_module: RpcModule<NodeClient>,
}

impl RpcServer {
    pub async fn new(rpc_port: usize, node_client: NodeClient) -> Result<Self> {

        let server = Server::builder()
            .set_message_buffer_capacity(50)
            .build(format!("0.0.0.0:{}", rpc_port))
            .await?;

        let rpc_module = RpcModule::new(node_client.clone());

        Ok(Self {
            server,
            node_client,
            rpc_module,
        })
    }

    pub fn add_route<R, Fut, Fun>(
        &mut self,
        method_name: &'static str,
        callback: Fun
    ) -> Result<&mut MethodCallback, RegisterMethodError>
    where
        R: IntoResponse + 'static,
        Fut: Future<Output = R> + Send,
        Fun: Fn(Params<'static>, Arc<NodeClient>, Extensions) -> Fut + Clone + Send + Sync + 'static,
    {
        self.rpc_module.register_async_method(method_name, callback)
    }

    // for mutable node_client
    pub fn add_route_mut<R, Fut, Fun>(
        &mut self,
        method_name: &'static str,
        callback: Fun
    ) -> Result<&mut MethodCallback, RegisterMethodError>
    where
        R: IntoResponse + 'static,
        Fut: Future<Output = R> + Send,
        Fun: Fn(Params<'static>, NodeClient, Extensions) -> Fut + Clone + Send + Sync + 'static,
    {
        let nc = self.node_client.clone();
        self.rpc_module.register_async_method(method_name, move |params, _, ext| {
            callback(params, nc.clone(), ext)
        })
    }

    pub async fn start(self) -> Result<SocketAddr> {
        let addr = self.server.local_addr()?;
        tokio::spawn(
            self.server
                .start(self.rpc_module)
                .stopped() // keep the server running, wait until stopped
        ).await?;
        Ok(addr)
    }
}

pub async fn run_server(rpc_port: usize, network_client: NodeClient) -> Result<SocketAddr> {

    let mut rpc_server = RpcServer::new(
        rpc_port,
        network_client.clone()
    ).await?;

    ////////////////////////////////////////////////////
    // RPC Endpoints
    ////////////////////////////////////////////////////

    rpc_server.add_route_mut(
        "spawn_agent",
        |params, mut nc, _| async move {

            let (
                agent_secrets_json,
                threshold,
                total_frags,
            ) = params.parse::<(AgentSecretsJson, usize, usize)>()?;

            nc.spawn_agent(
                    agent_secrets_json,
                    threshold,
                    total_frags,
                ).await.map_err(RpcError::from)
        }
    )?;

	rpc_server.add_route_mut(
        "trigger_node_failure",
        |_, mut nc, _| async move {
            nc.simulate_node_failure()
                .await.map_err(RpcError::from)
        }
    )?;

	rpc_server.add_route(
        "get_node_state",
        move |_, nc, _| async move {
            nc.get_node_state()
                .await.map_err(RpcError::from)
        }
    )?;

    rpc_server.add_route_mut(
        "spawn_memory_reverie",
        |params, mut nc, _| async move {

            let (
                memory_secrets_json,
                threshold,
                total_frags,
                access_public_key, // user address allowed to access the memory
            ) = params.parse::<(serde_json::Value, usize, usize, String)>()?;

            // Parse the string directly into an Address
            let access_public_key: Address = access_public_key.parse()
                .map_err(RpcError::from)?;

            nc.spawn_memory_reverie(
                memory_secrets_json,
                threshold,
                total_frags,
                AccessCondition::Ecdsa(access_public_key)
            ).await.map_err(RpcError::from)
        }
    )?;

    rpc_server.add_route_mut(
        "delegate_api_key",
        |params, mut nc, _| async move {
            let (
                reverie_id,
                reverie_type,
                access_key,
            ) = params.parse::<(ReverieId, ReverieType, AccessKey)>()?;

            nc.delegate_api_key(
                reverie_id,
                reverie_type,
                access_key,
            ).await.map_err(RpcError::from)
        }
    )?;

    rpc_server.add_route_mut(
        "execute_with_memory_reverie",
        |params, mut nc, _| async move {
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

            nc.execute_with_memory_reverie(
                reverie_id,
                reverie_type,
                access_key,
                anthropic_query,
            ).await.map_err(RpcError::from)
        }
    )?;

    rpc_server.add_route_mut(
        "report_usage",
        |params, mut nc, _| async move {

            let signed_usage_report = params.parse::<SignedUsageReport>()?;

            nc.report_usage(
                signed_usage_report,
            ).await.map_err(RpcError::from)
        }
    )?;

    ////////////////////////////////////////////////////
    // Subscriptions
    ////////////////////////////////////////////////////

    rpc_server.rpc_module.register_subscription(
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
    rpc_server.rpc_module.register_subscription(
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
                            "time": get_time_now(),
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
    // Start the server
    ////////////////////////////////////////////////////
    rpc_server.start().await
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
					None => break Err(RpcError("Subscription closed".to_string()).into()),
				};
				let msg = SubscriptionMessage::from_json(&item)?;
				match sink.try_send(msg) {
					Ok(_) => {},
					Err(TrySendError::Closed(e)) => {
                        break Err(RpcError("Subscription closed".to_string()).into())
                    },
					Err(TrySendError::Full(_)) => {},
				}
			}
		}
	}
}

pub fn get_time_now() -> std::time::Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("Time went backwards")
}

pub fn parse_tee_attestation_bytes(ta_bytes: Vec<u8>) -> serde_json::Value {
    let quote = runtime::tee_attestation::QuoteV4::from_bytes(&ta_bytes);
    let quote_body = match quote.quote_body {
        QuoteBody::SGXQuoteBody(e) => format!("{:?}", &e),
        QuoteBody::TD10QuoteBody(e) => format!("{:?}", &e),
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
        "time": get_time_now(),
    })
}

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

impl From<ErrReport> for RpcError {
    fn from(e: ErrReport) -> Self {
        RpcError(e.to_string())
    }
}

impl From<ErrorObject<'_>> for RpcError {
    fn from(e: ErrorObject<'_>) -> Self {
        RpcError(e.to_string())
    }
}

impl From<alloy_primitives::hex::FromHexError> for RpcError {
    fn from(e: alloy_primitives::hex::FromHexError) -> Self {
        RpcError(e.to_string())
    }
}
