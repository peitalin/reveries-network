use std::time::Instant;

use color_eyre::{owo_colors::OwoColorize, Result};
use futures::{
    AsyncRead,
    AsyncReadExt,
    AsyncWrite,
    AsyncWriteExt,
};
use serde::{Deserialize, Serialize};
use runtime::tee_attestation::QuoteV4;


const MSG_LEN_SIZE: u64 = 8;
const HEARTBEAT_MESSAGE_MAX_SIZE: u64 = 1024*24; // 24 kb
// HearbeatMessage contains a TDX QuoteV4 attestation, and block_height.
// TDX QuoteV4 is usually ~16kb (check)
// Append message length in the first 8 bytes (usize on 64-bit machines),

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

/// Takes in a stream. Waits to receive next `TeeAttestationBytes`
/// Returns the flushed stream and the received `TeeAttestationBytes`
pub(super) async fn receive_heartbeat_payload<S>(mut stream: S) -> Result<(S, TeeAttestation)>
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

    // trim payload to be msg_len and deserialize into a struct
    match serde_json::from_slice::<TeeAttestationBytes>(&payload) {
        Err(e) => {
            let payload_str = std::str::from_utf8(&payload)?;
            panic!("payload_str: {}\n>>> {}\n\tpayload_str.len(): {}\n\tpayload.len(): {}\n\tmsg_len: {}\n\tnum_bytes read: {}\n",
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
pub(super) async fn send_heartbeat_payload<S>(mut stream: S, tee_attestation: TeeAttestation) -> Result<S>
    where S: AsyncRead + AsyncWrite + Unpin,
{
    let msg_bytes = serde_json::to_vec(
        &TeeAttestationBytes::from(tee_attestation)
    ).unwrap();

    let msg_len = msg_bytes.len() as u64;
    if msg_len > HEARTBEAT_MESSAGE_MAX_SIZE {
        panic!(
            "Sending HeartbeatSig msg_bytes of length: {} exceeds HEARTBEAT_MESSAGE_MAX_SIZE: {}",
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