pub(crate) mod heartbeat_handler;

pub use heartbeat_handler::HeartbeatConfig;
use heartbeat_handler::TeeAttestation;

use std::{
    collections::VecDeque,
    task::Poll,
};
use color_eyre::{Result, eyre};
use tokio::sync::mpsc;
use futures::FutureExt;
use heartbeat_handler::{
    HeartbeatHandler,
    HeartbeatInEvent,
    HeartbeatOutEvent,
};
use libp2p::{
    core::{transport::PortUse, Endpoint},
    Multiaddr,
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
    PeerId,
};
use runtime::tee_attestation;
use runtime::tee_attestation::QuoteV4;


pub const HEARTBEAT_PROTOCOL: &str = "/1up/heartbeat/0.0.1";


#[derive(Debug)]
pub struct HeartbeatBehaviour {
    /// Config timers and max failure counts for Heartbeat
    pub(crate) config: HeartbeatConfig,
    /// for Heartbeat Behaviour to communicate with upper level EventLoop
    /// and disconnect from Swarm, or shutdown the LLM runtime.
    pub(crate) heartbeat_failure_sender: mpsc::Sender<String>,
    pending_events: VecDeque<HeartbeatAction>,
    /// This node's current heartbeat payload which will be broadcasted
    pub(crate) current_heartbeat_payload: TeeAttestation,
}

impl HeartbeatBehaviour {
    pub fn new(
        config: HeartbeatConfig,
        heartbeat_failure_sender: mpsc::Sender<String>,
    ) -> Self {
        Self {
            config,
            heartbeat_failure_sender,
            pending_events: VecDeque::default(),
            current_heartbeat_payload: TeeAttestation::default(),
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

    fn surface_shutdown_signal(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<()>> {
        // Surfaces shutdown signal to the heartbeat_failure_receiver channel in EventLoop
        async {
            self.heartbeat_failure_sender
                .send("HeartbeatFailure count too high! shutting down LLM runtime!".to_string())
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
        Ok(HeartbeatHandler::new(self.config.clone()))
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(HeartbeatHandler::new(self.config.clone()))
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
                self.pending_events
                    .push_back(HeartbeatAction::HeartbeatEvent(TeePayloadOutEvent {
                        peer_id,
                        latest_tee_attestation
                    }))
            }
            // Dispatch request for a Heartbeat from other Peers
            HeartbeatOutEvent::RequestHeartbeat => {
                // push onto pending_events, which will be poll()'d and executed
                self.pending_events
                    .push_back(HeartbeatAction::HeartbeatRequest  {
                        peer_id,
                        connection_id,
                        in_event: HeartbeatInEvent::LatestHeartbeat(
                            self.current_heartbeat_payload.clone(),
                        ),
                    })
            }
            HeartbeatOutEvent::HeartbeatFailure => {
                // bubble up some command to EventLoop to shut the LLM Runtime down
                // push HeartbeatFailure to pending events for async processing
                self.pending_events
                    .push_back(HeartbeatAction::HeartbeatFailure)
            }
            HeartbeatOutEvent::IncreaseBlockHeight => {

                let (
                    _tee_quote ,
                    tee_quote_bytes
                ) = tee_attestation::generate_tee_attestation(false)
                    .expect("tee attestation generation err");

                self.set_tee_attestation(tee_quote_bytes);
                self.increment_block_height();
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

                HeartbeatAction::HeartbeatFailure => {
                    // Send shutdown signal to EventLoop instead of ToSwarm
                    let _ = self.surface_shutdown_signal(cx);

                    // // return pending to async runtime and continue
                    return Poll::Pending
                }
            }
        };

        Poll::Pending
    }

}


#[derive(Debug, Clone)]
enum HeartbeatAction {
    HeartbeatEvent(TeePayloadOutEvent),
    HeartbeatRequest {
        peer_id: PeerId,
        connection_id: ConnectionId,
        in_event: HeartbeatInEvent,
    },
    HeartbeatFailure
}

#[derive(Debug, Clone)]
pub struct TeePayloadOutEvent {
    pub peer_id: PeerId,
    pub latest_tee_attestation: TeeAttestation,
}
