use super::HEARTBEAT_PROTOCOL;
use futures::{
    future::BoxFuture,
    AsyncRead,
    AsyncReadExt,
    AsyncWrite,
    AsyncWriteExt,
    FutureExt,
};
// use std::sync::{mpsc};
use futures::channel::{mpsc, oneshot};
use futures::SinkExt;
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
use std::{
    num::NonZeroU32,
    pin::Pin,
    task::Poll,
    time::Duration,
};
use anyhow::{Result, Error};
use tokio::time::{
    sleep,
    Sleep,
};
use tracing::debug;
use runtime::tee_attestation::QuoteV4;


#[derive(Debug, Clone)]
pub enum HeartbeatInEvent {
    LatestHeartbeat(TeeAttestation),
}

#[derive(Debug, Clone)]
pub enum HeartbeatOutEvent {
    HeartbeatPayload(TeeAttestation),
    RequestHeartbeat,
}

#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Sending of `TeeAttestation` should not take longer than this
    send_timeout: Duration,
    /// Idle time before sending next `TeeAttestation`
    idle_timeout: Duration,
    /// Max failures allowed.
    /// If reached `HeartbeatHandler` will request closing of the connection.
    max_failures: NonZeroU32,
    /// for Heartbeat Behaviour to communicate with upper level EventLoop
    pub sender: mpsc::Sender<String>,
}

impl HeartbeatConfig {
    pub fn new(
        send_timeout: Duration,
        idle_timeout: Duration,
        max_failures: NonZeroU32,
        sender: mpsc::Sender<String>
    ) -> Self {
        Self {
            send_timeout,
            idle_timeout,
            max_failures,
            sender,
        }
    }
}

// impl Default for HeartbeatConfig {
//     fn default() -> Self {
//         Self::new(
//             Duration::from_secs(60),
//             Duration::from_secs(1),
//             NonZeroU32::new(5).expect("5 != 0"),
//             None,
//         )
//     }
// }

#[derive(Debug, Clone)]
pub struct TeeAttestation {
    /// QuoteV4 does not have Serializer trait, need to store byte representation
    pub tee_attestation: Option<QuoteV4>,
    pub tee_attestation_bytes: Option<Vec<u8>>,
    pub block_height: u32,
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
    failure_count: u32,
}

impl HeartbeatHandler {
    pub fn new(config: HeartbeatConfig) -> Self {
        Self {
            config,
            inbound: None,
            outbound: None,
            timer: Box::pin(sleep(Duration::new(0, 0))),
            failure_count: 0,
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
            // TODO: Close connection properly
            // if self.failure_count >= self.config.max_failures.into() {
            //     // Request from `Swarm` to close the faulty connection
            //     return Poll::Ready(ConnectionHandlerEvent::Close(
            //         HeartbeatFailure::Timeout,
            //     ))
            // }

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
                                self.failure_count = self.failure_count.saturating_add(1);
                                debug!(target: "1up", "Sending Heartbeat timed out, this is {} time it failed with this connection", self.failure_count);
                            } else {
                                self.outbound = Some(OutboundState::SendingTeeAttestation(
                                    outbound_block_height,
                                ));
                                break
                            }
                        }
                        Poll::Ready(Ok(stream)) => {
                            // reset failure count
                            self.failure_count = 0;

                            let _ = async {
                                self.config.sender
                                    .send("Callback after reset HB timer".to_string()).await
                            }.boxed().poll_unpin(cx);

                            // start new idle timeout until next request & send
                            self.timer = Box::pin(sleep(self.config.idle_timeout));
                            self.outbound = Some(OutboundState::Idle(stream));
                        }
                        Poll::Ready(Err(_)) => {
                            self.failure_count = self.failure_count.saturating_add(1);
                            debug!(
                                target: "1up", "Sending Heartbeat failed, {}/{} failures for this connection",
                                self.failure_count,
                                self.config.max_failures
                            );
                        }
                    }
                }
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
                self.failure_count = self.failure_count.saturating_add(1);
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
    debug!("receiver: msg_len: {:?}", msg_len);

    // trim payload to be msg_len and deserialize into a struct
    match serde_json::from_slice::<TeeAttestationBytes>(&payload) {
        Err(e) => {
            let payload_str = &std::str::from_utf8(&payload)?;
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
