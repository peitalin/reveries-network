use super::HEARTBEAT_PROTOCOL;

use std::{
    pin::Pin,
    task::Poll,
    time::Duration,
};
use color_eyre::{Result, eyre::Error};
use futures::{
    future::BoxFuture,
    AsyncRead,
    AsyncReadExt,
    AsyncWrite,
    AsyncWriteExt,
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
use serde::{Deserialize, Serialize};
use tokio::time::{
    sleep,
    Sleep,
};
use tracing::debug;
use runtime::tee_attestation::QuoteV4;
use super::HeartbeatConfig;


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


#[derive(Debug, Clone)]
pub struct TeeAttestation {
    /// QuoteV4 does not have Serializer trait, need to store byte representation
    pub tee_attestation: Option<QuoteV4>,
    pub tee_attestation_bytes: Option<Vec<u8>>,
    pub block_height: u32,
}

impl Default for TeeAttestation {
    fn default() -> Self {
        Self {
            tee_attestation: None,
            tee_attestation_bytes: None,
            block_height: 1,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TeeAttestationBytes {
    pub tee_attestation_bytes: Option<Vec<u8>>,
    pub block_height: u32,
}
impl From<TeeAttestation> for TeeAttestationBytes {
    fn from(value: TeeAttestation) -> Self {
        Self {
            tee_attestation_bytes: value.tee_attestation_bytes,
            block_height: value.block_height
        }
    }
}
impl From<TeeAttestationBytes> for TeeAttestation {
    fn from(value: TeeAttestationBytes) -> Self {
        match value.tee_attestation_bytes {
            None => {
                Self {
                    tee_attestation: None,
                    tee_attestation_bytes: value.tee_attestation_bytes,
                    block_height: value.block_height
                }
            }
            Some(ta_bytes) => {
                Self {
                    tee_attestation: Some(QuoteV4::from_bytes(&ta_bytes)),
                    tee_attestation_bytes: Some(ta_bytes),
                    block_height: value.block_height
                }
            }
        }
    }
}


type InboundData = BoxFuture<'static, Result<(Stream, TeeAttestation), Error>>;
type OutboundData = BoxFuture<'static, Result<Stream, Error>>;

pub struct HeartbeatHandler {
    config: HeartbeatConfig,
    inbound: Option<InboundData>,
    outbound: Option<OutboundState>,
    timer: Pin<Box<Sleep>>,
    // Internal failure count for the node to keep track of when it should shutdown it's LLM runtime.
    // It is not related to the PRE re-incarnation protocol--that is determined by external nodes
    // after they don't hear from this node for a while.
    internal_fail_count: u32,
}

impl HeartbeatHandler {
    pub fn new(config: HeartbeatConfig) -> Self {
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
                // Dispatch message to on_connection_handler_event handler to reboot, and
                // reset from Vessel mode to MPC mode.
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
                                debug!(target: "1up", "Sending Heartbeat timed out, this is {} time it failed with this connection", self.internal_fail_count);
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


const MSG_LEN_SIZE: u64 = 8;
const HEARTBEAT_MESSAGE_MAX_SIZE: u64 = 1024*24; // 24 kb
// HearbeatMessage contains a TDX QuoteV4 attestation, and block_height.
// TDX QuoteV4 is usually ~16kb (check)
// Append message length in the first 8 bytes (usize on 64-bit machines),

/// Takes in a stream. Waits to receive next `TeeAttestationBytes`
/// Returns the flushed stream and the received `TeeAttestationBytes`
async fn receive_heartbeat_payload<S>(mut stream: S) -> Result<(S, TeeAttestation)>
    where S: AsyncRead + AsyncWrite + Unpin,
{
    let mut msg_len_array = [0u8; MSG_LEN_SIZE as usize];

    // read first 8 bytes with read_exact and consume first 8 bytes
    stream.read_exact(&mut msg_len_array).await?;
    stream.flush().await?;
    // parse the msg length
    let msg_len: u64 = u64::from_be_bytes(msg_len_array);

    // then read the rest of the payload
    let mut payload = vec![0u8; msg_len as usize];
    stream.read_exact(&mut payload).await?;
    stream.flush().await?;
    let num_bytes_read = payload.len() as u64;

    // println!("msg_len_array: {:?}", msg_len_array);
    // debug!("receiver: msg_len: {:?}", msg_len);

    // trim payload to be msg_len and deserialize into a struct
    match serde_json::from_slice::<TeeAttestationBytes>(&payload) {
        Err(e) => {
            let payload_str = std::str::from_utf8(&payload)?;
            panic!("\npayload_str: {}\n\n>>> {}\n\tpayload_str.len(): {}\n\tpayload.len(): {}\n\tmsg_len: {}\n\tnum_bytes read: {}\n",
                payload_str,
                e,
                payload_str.len(),
                payload.len(),
                msg_len,
                num_bytes_read
            );
        }
        Ok(tee_attestation_bytes) => {
            let tee_attestation = TeeAttestation::from(tee_attestation_bytes);
            Ok((stream, tee_attestation))
        }
    }
}

/// Takes in a stream and latest `TeeAttestation`
/// Sends the `TeeAttestation` and returns back the stream after flushing it
async fn send_heartbeat_payload<S>(mut stream: S, tee_attestation: TeeAttestation) -> Result<S>
    where S: AsyncRead + AsyncWrite + Unpin,
{
    let msg_bytes = serde_json::to_vec(
        &TeeAttestationBytes::from(tee_attestation)
    ).unwrap();

    let msg_len = msg_bytes.len() as u64;
    if msg_len > HEARTBEAT_MESSAGE_MAX_SIZE {
        panic!(
            "HeartbeatSig msg_bytes of length: {} exceeds HEARTBEAT_MESSAGE_MAX_SIZE: {}",
            msg_len,
            HEARTBEAT_MESSAGE_MAX_SIZE
        );
    };

    // Append msg_len to first 8 bytes of the message
    let msg_len_bytes: [u8; MSG_LEN_SIZE as usize] = msg_len.to_be_bytes();
    let full_msg_bytes: Vec<u8> = [msg_len_bytes.to_vec(), msg_bytes].concat();

    stream.write_all(&full_msg_bytes).await?;
    stream.flush().await?;
    Ok(stream)
}
