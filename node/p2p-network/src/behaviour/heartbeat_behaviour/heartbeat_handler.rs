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
use super::tee_quote_parser::{
    receive_heartbeat_payload,
    send_heartbeat_payload
};


#[derive(Debug, Clone)]
pub enum HeartbeatInEvent {
    LatestHeartbeat(TeeAttestation),
}

#[derive(Debug, Clone)]
pub enum HeartbeatOutEvent {
    HeartbeatPayload(TeeAttestation),
    RequestHeartbeat,
    FailedToSendHeartbeat,
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
    timer: Pin<Box<Sleep>>,
    // Internal failure count for the node to keep track of when it should shutdown it's runtime.
    // It is not related to the PRE re-incarnation protocol--that is determined by external nodes
    // after they don't hear from this node for a while.
    internal_fail_count: u32,
}

impl HeartbeatHandler {
    pub fn new(
        config: HeartbeatConfig,
        internal_fail_count: std::sync::Arc<tokio::sync::Mutex<u32>>
    ) -> Self {
        Self {
            config,
            inbound: None,
            outbound: None,
            timer: Box::pin(sleep(Duration::new(0, 0))),
            internal_fail_count: 0,
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
    ) -> std::task::Poll<
        ConnectionHandlerEvent<
            Self::OutboundProtocol,
            Self::OutboundOpenInfo,
            Self::ToBehaviour,
        >,
    > {
        if let Some(inbound_stream_and_block_height) = self.inbound.as_mut() {
            match inbound_stream_and_block_height.poll_unpin(cx) {
                Poll::Ready(Err(_)) => {
                    debug!(target: "1up", "Incoming heartbeat errored");
                    self.inbound = None;
                }
                Poll::Ready(Ok((stream, tee_attestation))) => {
                    // start waiting for the next `TeeAttestation`
                    self.inbound = Some(receive_heartbeat_payload(stream).boxed());

                    // report newly received `TeeAttestation` to the Behaviour
                    return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                        HeartbeatOutEvent::HeartbeatPayload(tee_attestation),
                    ))
                }
                _ => {}
            }
        }

        loop {
            // `ConnectionHandlerEvent::Close` was removed.
            // To close a connection, use ToSwarm::CloseConnection or Swarm::close_connection.
            // See https://github.com/libp2p/rust-libp2p/issues/3591
            if self.internal_fail_count >= self.config.max_failures.into() {
                // Unable to send HB out to other peers.
                // Dispatch message to on_connection_handler_event handler to reboot
                return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                    HeartbeatOutEvent::FailedToSendHeartbeat
                ))
            }

            match self.outbound.take() {

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

                Some(OutboundState::SendingTeeAttestation(
                    mut outbound_block_height
                )) => {
                    match outbound_block_height.poll_unpin(cx) {
                        Poll::Pending => {
                            if self.timer.poll_unpin(cx).is_ready() {
                                // Time for successful send expired!

                                self.internal_fail_count = self.internal_fail_count.saturating_add(1);
                                debug!(
                                    target: "1up",
                                    "Sending Heartbeat timed out, failed {} time(s) with this connection",
                                    self.internal_fail_count

                                );
                            } else {
                                self.outbound = Some(OutboundState::SendingTeeAttestation(
                                    outbound_block_height,
                                ));
                                break
                            }
                        }
                        Poll::Ready(Ok(stream)) => {
                            // reset failure count
                            self.internal_fail_count = 0;

                            // start new idle timeout until next request & send
                            self.timer = Box::pin(sleep(self.config.idle_timeout));
                            self.outbound = Some(OutboundState::Idle(stream));
                        }
                        Poll::Ready(Err(_)) => {
                            self.internal_fail_count = self.internal_fail_count.saturating_add(1);
                            debug!(
                                target: "1up", "Sending Heartbeat failed, {}/{} failures for this connection",
                                self.internal_fail_count,
                                self.config.max_failures
                            );
                        }
                    }
                }

                // Poll timer to see if we can make the next TEE Heartbeat Request
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

                    let protocol =
                        SubstreamProtocol::new(ReadyUpgrade::new(HEARTBEAT_PROTOCOL), ())
                            .with_timeout(self.config.send_timeout);

                    return Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest {
                        protocol,
                    })
                }
            }
        }
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
                    send_heartbeat_payload(stream, heartbeat_payload).boxed()
                ))
            }
            other_state => self.outbound = other_state,
        }
    }

    fn on_connection_event(
        &mut self,
        event: ConnectionEvent<
            Self::InboundProtocol,
            Self::OutboundProtocol,
            Self::InboundOpenInfo,
            Self::OutboundOpenInfo,
        >,
    ) {
        match event {
            ConnectionEvent::FullyNegotiatedInbound(FullyNegotiatedInbound {
                protocol: stream,
                ..
            }) => {
                self.inbound = Some(receive_heartbeat_payload(stream).boxed());
            }
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: stream,
                ..
            }) => {
                self.outbound = Some(OutboundState::RequestingTeeHeartbeat {
                    stream,
                    requested: false,
                })
            }
            ConnectionEvent::DialUpgradeError(_) => {
                self.outbound = None;
                self.internal_fail_count = self.internal_fail_count.saturating_add(1);
            }
            _ => {}
        }
    }
}
