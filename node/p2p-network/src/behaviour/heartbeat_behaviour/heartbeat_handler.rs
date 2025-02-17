use super::HEARTBEAT_PROTOCOL;

use std::{
    pin::Pin,
    task::Poll,
    time::Duration,
};
use color_eyre::{Result, eyre::Error};
use futures::{
    future::BoxFuture,
    FutureExt,
};
use libp2p::{
    core::upgrade::ReadyUpgrade,
    swarm::{
        handler::{
            ConnectionEvent,
            FullyNegotiatedInbound,
            FullyNegotiatedOutbound,
        },
        ConnectionHandler,
        ConnectionHandlerEvent,
        Stream,
        SubstreamProtocol,
    },
};
use tokio::time::{
    sleep,
    Sleep,
};
use tracing::debug;
use super::{HeartbeatConfig, TeeAttestation};
use super::tee_quote_parser;


#[derive(Debug, Clone)]
pub enum HeartbeatInEvent {
    LatestHeartbeat(TeeAttestation),
}

#[derive(Debug, Clone)]
pub enum HeartbeatOutEvent {
    HeartbeatPayload(TeeAttestation),
    RequestHeartbeat,
    ResetFailureCount,
    IncrementFailureCount(u32),
    GenerateTeeAttestation
}

/// Represents state of the Oubound stream
enum OutboundState {
    Idle(Stream),
    NegotiatingStream,
    RequestingTeeHeartbeat {
        stream: Stream,
        /// `false` if the TeeAttestation has not been requested yet.
        /// `true` if the TeeAttestation has been requested in the current `Heartbeat` cycle.
        requested: bool,
    },
    SendingTeeAttestation(OutboundData),
}

type InboundData = BoxFuture<'static, Result<(Stream, TeeAttestation), Error>>;
type OutboundData = BoxFuture<'static, Result<Stream, Error>>;

pub struct HeartbeatHandler {
    config: HeartbeatConfig,
    inbound: Option<InboundData>,
    outbound: Option<OutboundState>,
    dial_failures: Option<u32>,
    timer: Pin<Box<Sleep>>,
}

impl HeartbeatHandler {
    pub fn new(config: HeartbeatConfig) -> Self {
        Self {
            config,
            inbound: None,
            outbound: None,
            dial_failures: None,
            timer: Box::pin(sleep(Duration::new(0, 0))),
        }
    }
}

impl ConnectionHandler for HeartbeatHandler {
    type FromBehaviour = HeartbeatInEvent;
    type ToBehaviour = HeartbeatOutEvent;
    type InboundProtocol = ReadyUpgrade<&'static str>;
    type OutboundProtocol = ReadyUpgrade<&'static str>;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<ReadyUpgrade<&'static str>, ()> {
        SubstreamProtocol::new(ReadyUpgrade::new(HEARTBEAT_PROTOCOL), ())
    }

    fn connection_keep_alive(&self) -> bool {
        // Heartbeat protocol wants to keep the connection alive
        true
    }

    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<ConnectionHandlerEvent<Self::OutboundProtocol, (), Self::ToBehaviour>> {

        //// Inbound Events
        if let Some(inbound_stream) = self.inbound.as_mut() {
            match inbound_stream.poll_unpin(cx) {
                Poll::Ready(Err(_)) => {
                    debug!(target: "1up", "Incoming heartbeat errored");
                    self.inbound = None;
                }
                Poll::Ready(Ok((stream, tee_attestation))) => {
                    // start waiting for the next `TeeAttestation`
                    self.inbound = Some(tee_quote_parser::receive_heartbeat_payload(stream).boxed());
                    // report newly received peer `TeeAttestation` to heartbeat_behaviour/mod.rs
                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                        HeartbeatOutEvent::HeartbeatPayload(tee_attestation),
                    ))
                }
                _ => {}
            }
        }

        if let Some(dial_fail_count) = self.dial_failures {
            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                HeartbeatOutEvent::IncrementFailureCount(dial_fail_count)
            ))
        }

        //// Outbound Events
        loop {
            match self.outbound.take() {
                // Requesting TEE Heartbeat
                Some(OutboundState::RequestingTeeHeartbeat { requested, stream }) => {
                    self.outbound = Some(OutboundState::RequestingTeeHeartbeat {
                        stream,
                        requested: true,
                    });
                    if !requested {
                        return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                            HeartbeatOutEvent::RequestHeartbeat,
                        ))
                    }
                    break
                }

                // Sending TEE heartbeat
                Some(OutboundState::SendingTeeAttestation(
                    mut outbound_block_height
                )) => {
                    match outbound_block_height.poll_unpin(cx) {
                        Poll::Pending => {
                            if self.timer.poll_unpin(cx).is_ready() {
                                // Time for successful send expired, increment failure count
                                return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                    HeartbeatOutEvent::IncrementFailureCount(1),
                                ))
                            } else {
                                self.outbound = Some(OutboundState::SendingTeeAttestation(
                                    outbound_block_height,
                                ));
                                break
                            }
                        }
                        Poll::Ready(Ok(stream)) => {
                            // start new idle timeout until next request and send
                            self.timer = Box::pin(sleep(self.config.idle_timeout));
                            self.outbound = Some(OutboundState::Idle(stream));
                            // reset failure count
                            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                HeartbeatOutEvent::ResetFailureCount,
                            ))
                        }
                        Poll::Ready(Err(_)) => {
                            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                                HeartbeatOutEvent::IncrementFailureCount(1),
                            ))
                        }
                    }
                }

                // Idle: Poll timer to see if we can make the next TEE Heartbeat Request
                Some(OutboundState::Idle(stream)) => match self.timer.poll_unpin(cx) {
                    Poll::Pending => {
                        self.outbound = Some(OutboundState::Idle(stream));
                        break
                    }
                    Poll::Ready(()) => {
                        self.outbound = Some(OutboundState::RequestingTeeHeartbeat {
                            stream,
                            requested: false,
                        });
                        // generate new TEE attestation
                        return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                            HeartbeatOutEvent::GenerateTeeAttestation
                        ));
                    }
                },

                Some(OutboundState::NegotiatingStream) => {
                    self.outbound = Some(OutboundState::NegotiatingStream);
                    break
                }

                None => {
                    // Request new stream
                    self.outbound = Some(OutboundState::NegotiatingStream);

                    return Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest {
                        protocol: SubstreamProtocol::new(ReadyUpgrade::new(HEARTBEAT_PROTOCOL), ())
                            .with_timeout(self.config.send_timeout)
                    })
                }
            }
        }
        // Otherwise return Pending
        Poll::Pending
    }

    fn on_behaviour_event(&mut self, event: Self::FromBehaviour) {
        let HeartbeatInEvent::LatestHeartbeat(heartbeat_payload) = event;

        match self.outbound.take() {
            // Respond to Heartbeat Request
            Some(OutboundState::RequestingTeeHeartbeat {
                requested: true,
                stream,
            }) => {
                // start new send timeout
                self.timer = Box::pin(sleep(self.config.send_timeout));
                // send latest `TeeAttestation`
                self.outbound = Some(OutboundState::SendingTeeAttestation(
                    tee_quote_parser::send_heartbeat_payload(stream, heartbeat_payload).boxed()
                ))
            }
            other_state => self.outbound = other_state,
        }
    }

    fn on_connection_event(
        &mut self,
        event: ConnectionEvent<Self::InboundProtocol, Self::OutboundProtocol>,
    ) {
        match event {
            ConnectionEvent::FullyNegotiatedInbound(FullyNegotiatedInbound {
                protocol: stream, ..
            }) => {
                self.inbound = Some(tee_quote_parser::receive_heartbeat_payload(stream).boxed());
            }
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: stream, ..
            }) => {
                self.outbound = Some(OutboundState::RequestingTeeHeartbeat {
                    stream,
                    requested: false,
                })
            }
            ConnectionEvent::DialUpgradeError(_) => {
                self.outbound = None;
                // surface failure to heartbeat_behaviour/mod.rs via poll()
                self.dial_failures = Some(1);
            }
            _ => {}
        }
    }
}
