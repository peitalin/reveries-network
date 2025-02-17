pub(crate) mod heartbeat_handler;
mod config;
mod tee_quote_parser;

use std::collections::VecDeque;
use std::task::Poll;
use std::sync::Arc;
use color_eyre::{Result, eyre};
use colored::Colorize;
use futures::FutureExt;
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        derive_prelude::ConnectionId,
        ConnectionDenied,
        FromSwarm,
        NetworkBehaviour,
        NotifyHandler,
        THandler,
        THandlerInEvent,
        THandlerOutEvent,
        ToSwarm,
    },
    Multiaddr,
    PeerId,
};
use tokio::sync::mpsc;
use tracing::debug;

pub use config::HeartbeatConfig;
use heartbeat_handler::{
    HeartbeatHandler,
    HeartbeatInEvent,
    HeartbeatOutEvent,
};
use runtime::tee_attestation;
use runtime::tee_attestation::QuoteV4;
pub use tee_quote_parser::TeeAttestation;


pub const HEARTBEAT_PROTOCOL: &str = "/1up/heartbeat/0.0.1";

#[derive(Debug, Clone)]
enum HeartbeatAction {
    HeartbeatEvent(TeePayloadOutEvent),
    HeartbeatRequest {
        peer_id: PeerId,
        connection_id: ConnectionId,
        in_event: HeartbeatInEvent,
    },
    ShutdownIfMaxFailuresExceeded
}

#[derive(Debug, Clone)]
pub struct TeePayloadOutEvent {
    pub peer_id: PeerId,
    pub latest_tee_attestation: TeeAttestation,
}

#[derive(Debug)]
pub struct HeartbeatBehaviour {
    /// Config timers and max failure counts for Heartbeat
    pub(crate) config: HeartbeatConfig,

    /// for Heartbeat Behaviour to communicate with upper level EventLoop
    /// and disconnect from Swarm, or shutdown the LLM runtime.
    pub(crate) internal_heartbeat_fail_sender: mpsc::Sender<HeartbeatConfig>,

    pub(crate) heartbeat_sender: async_channel::Sender<Option<Vec<u8>>>,

    pending_events: VecDeque<HeartbeatAction>,

    /// This node's current heartbeat payload which will be broadcasted
    pub(crate) current_heartbeat_payload: TeeAttestation,

    /// internal heartbeat fail count to know when to shutdown runtime and reboot container
    // It is not related to the PRE re-incarnation protocol--that is determined by external nodes
    // after they don't hear from this node for a while.
    pub(crate) internal_fail_count: std::sync::Arc<u32>,
}

impl HeartbeatBehaviour {
    pub fn new(
        config: HeartbeatConfig,
        internal_heartbeat_fail_sender: mpsc::Sender<HeartbeatConfig>,
        heartbeat_sender: async_channel::Sender<Option<Vec<u8>>>,
    ) -> Self {
        Self {
            config,
            internal_heartbeat_fail_sender,
            heartbeat_sender,
            pending_events: VecDeque::default(),
            current_heartbeat_payload: TeeAttestation::default(),
            internal_fail_count: std::sync::Arc::new(0),
        }
    }

    pub fn set_tee_attestation(&mut self, tee_attestation: Vec<u8>) {
        let quote = QuoteV4::from_bytes(&tee_attestation);
        // can't deserialize QuoteV4 back to bytes (unless we fork the lib), so save both.
        self.current_heartbeat_payload.tee_attestation = Some(quote);
        self.current_heartbeat_payload.tee_attestation_bytes = Some(tee_attestation);
    }

    pub fn set_heartbeat_payload(&mut self, heartbeat_payload: TeeAttestation) {
        self.current_heartbeat_payload = heartbeat_payload;
    }

    pub fn increment_block_height(&mut self) {
        self.current_heartbeat_payload.block_height += 1;
    }

    pub fn is_non_production_environment(&self) -> bool {
        std::env::var("ENV").unwrap_or("".to_string()) != "production".to_string()
    }

    pub(crate) async fn trigger_heartbeat_failure(&mut self) {
        if self.is_non_production_environment() {
            self.internal_fail_count = 10_000.into();
            self.pending_events.push_back(HeartbeatAction::ShutdownIfMaxFailuresExceeded);
        } else {
            println!("Trigger heartbeat failure only works in non production environments");
        }
    }

    fn surface_shutdown_signal_to_container_manager(
        &mut self,
        cx: &mut std::task::Context<'_>
    ) -> Poll<Result<()>> {
        debug!("surface_shutdown_signal() to heartbeat_fail_receiver in NetworkEvents");
        // Surfaces shutdown signal to the heartbeat_fail_receiver channel in NetworkEvents
        async {
            self.internal_heartbeat_fail_sender
                .send(
                    self.config.clone()
                )
                .await
                .map_err(|e| eyre::anyhow!(e.to_string()))
        }
        .boxed()
        .poll_unpin(cx)
    }
}

impl NetworkBehaviour for HeartbeatBehaviour {
    type ConnectionHandler = HeartbeatHandler;
    type ToSwarm = TeePayloadOutEvent;

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(HeartbeatHandler::new(
            self.config.clone()
        ))
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(HeartbeatHandler::new(
            self.config.clone()
        ))
    }

    fn on_swarm_event(&mut self, _event: FromSwarm) {}

    // heartbeat_handler.rs propagates poll() events up to this handler
    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        match event {
            // Incoming Heartbeats from other Peers
            HeartbeatOutEvent::HeartbeatPayload(latest_tee_attestation) => {
                // push onto pending_events, which will be poll()'d and executed
                self.pending_events.push_back(
                    HeartbeatAction::HeartbeatEvent(TeePayloadOutEvent {
                        peer_id,
                        latest_tee_attestation
                    })
                );
            }
            HeartbeatOutEvent::ResetFailureCount => {
                self.internal_fail_count = 0.into();
            }
            // Dispatch request for a Heartbeat from other Peers
            HeartbeatOutEvent::RequestHeartbeat => {
                // push onto pending_events, which will be poll()'d and executed
                self.pending_events.push_back(
                    HeartbeatAction::HeartbeatRequest  {
                        peer_id,
                        connection_id,
                        in_event: HeartbeatInEvent::LatestHeartbeat(
                            self.current_heartbeat_payload.clone(),
                        ),
                    }
                )
            }
            HeartbeatOutEvent::IncrementFailureCount(n) => {
                // bubble up some command to EventLoop to shut the Runtime down
                // push ShutdownIfMaxFailuresExceeded to pending events for async processing
                self.internal_fail_count = (self.internal_fail_count.saturating_add(n)).into();

                debug!(target: "heartbeat",
                    "Sending Heartbeat timed out, failed {} time(s) with this connection",
                    self.internal_fail_count
                );

                self.pending_events.push_back(HeartbeatAction::ShutdownIfMaxFailuresExceeded);

                debug!(target: "heartbeat", "Sending Heartbeat failed, {}/{} failures for this connection",
                    self.internal_fail_count,
                    self.config.max_failures
                );
            }
            HeartbeatOutEvent::GenerateTeeAttestation => {

                let (
                    _tee_quote ,
                    tee_quote_bytes
                ) = tee_attestation::generate_tee_attestation(false)
                    .expect("TEE attestation generation error");

                self.set_tee_attestation(tee_quote_bytes);
            }
        }
    }

    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {

        if let Some(action) = self.pending_events.pop_front() {
            match action {

                HeartbeatAction::HeartbeatEvent(event) => {

                    // send to NodeClient, expose stream to subscribers
                    let _ = async {
                        self.heartbeat_sender.send(
                            event.latest_tee_attestation.tee_attestation_bytes.clone()
                        ).await
                    }
                    .boxed()
                    .poll_unpin(cx);

                    return Poll::Ready(ToSwarm::GenerateEvent(event))
                },

                HeartbeatAction::HeartbeatRequest  {
                    peer_id,
                    connection_id,
                    in_event,
                } => {
                    return Poll::Ready(ToSwarm::NotifyHandler {
                        handler: NotifyHandler::One(connection_id),
                        peer_id,
                        event: in_event,
                    })
                }

                HeartbeatAction::ShutdownIfMaxFailuresExceeded => {
                    // Send shutdown signal to NetworkEvents. Only this node knows
                    // it's run into an error. Peers will have to wait until hearbeats
                    // start to timeout before initiating reincarnation process
                    tracing::error!("ShutdownIfMaxFailuresExceeded: {}",
                        format!("internal_fail_count({}) > max_failures({})",
                            self.internal_fail_count,
                            self.config.max_failures
                        )
                    );

                    if self.internal_fail_count.as_ref() > &self.config.max_failures {
                        let _ = self.surface_shutdown_signal_to_container_manager(cx);
                    }
                    // return pending to async runtime and continue
                    return Poll::Pending
                }
            }
        };

        Poll::Pending
    }

}

