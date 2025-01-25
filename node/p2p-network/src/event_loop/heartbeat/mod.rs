pub(crate) mod heartbeat_handler;

pub use heartbeat_handler::Config;

use std::{
    collections::VecDeque,
    task::Poll,
};
use runtime::tee_attestation::QuoteV4;
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
use heartbeat_handler::TeeAttestation;


pub const HEARTBEAT_PROTOCOL: &str = "/nirvana/heartbeat/0.0.1";

#[derive(Debug, Clone)]
enum HeartbeatAction {
    HeartbeatEvent(TeePayloadOutEvent),
    HeartbeatRequest {
        peer_id: PeerId,
        connection_id: ConnectionId,
        in_event: HeartbeatInEvent,
    },
}

impl HeartbeatAction {
    fn build(self) -> ToSwarm<TeePayloadOutEvent, HeartbeatInEvent> {
        match self {
            Self::HeartbeatEvent(event) => ToSwarm::GenerateEvent(event),
            Self::HeartbeatRequest  {
                peer_id,
                connection_id,
                in_event,
            } => ToSwarm::NotifyHandler {
                handler: NotifyHandler::One(connection_id),
                peer_id,
                event: in_event,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct TeePayloadOutEvent {
    pub peer_id: PeerId,
    pub latest_tee_attestation: TeeAttestation,
}

#[derive(Debug, Clone)]
pub struct Behaviour {
    config: Config,
    pending_events: VecDeque<HeartbeatAction>,
    pub current_heartbeat_payload: TeeAttestation,
}

impl Behaviour {
    pub fn new(config: Config, heartbeat_payload: TeeAttestation) -> Self {
        Self {
            config,
            pending_events: VecDeque::default(),
            current_heartbeat_payload: heartbeat_payload,
        }
    }

    pub fn set_tee_attestation(&mut self, tee_attestation: Vec<u8>) {
        let quote = QuoteV4::from_bytes(&tee_attestation);
        self.current_heartbeat_payload.tee_attestation = Some(quote);
        self.current_heartbeat_payload.tee_attestation_bytes = Some(tee_attestation);
    }

    pub fn set_heartbeat_payload(&mut self, heartbeat_payload: TeeAttestation) {
        self.current_heartbeat_payload = heartbeat_payload;
    }

    pub fn increment_block_height(&mut self) {
        self.current_heartbeat_payload.block_height += 1;
    }
}

impl NetworkBehaviour for Behaviour {
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

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        // println!("1>> connection_handler_event: peerId {} event: {:?}", crate::short_peer_id(peer_id), event);
        match event {
            HeartbeatOutEvent::HeartbeatPayload(latest_tee_attestation) => {
                self.pending_events
                    .push_back(HeartbeatAction::HeartbeatEvent(TeePayloadOutEvent {
                        peer_id,
                        latest_tee_attestation
                    }))
            }
            HeartbeatOutEvent::RequestHeartbeat => {
                self.pending_events
                    .push_back(HeartbeatAction::HeartbeatRequest  {
                        peer_id,
                        connection_id,
                        in_event: HeartbeatInEvent::LatestHeartbeat(
                            self.current_heartbeat_payload.clone(),
                        ),
                    })
            }
        }
    }

    fn poll(
        &mut self,
        _: &mut std::task::Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        if let Some(action) = self.pending_events.pop_front() {
            return Poll::Ready(action.build())
        }

        Poll::Pending
    }
}