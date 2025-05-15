use serde::{Deserialize, Serialize};

// This struct is sent by llm-proxy to p2p-node
// and also needs to be understood by p2p-node when deserializing.
// (p2p-node will also need to derive Deserialize for its copy or if it uses this crate directly)
#[derive(Debug, Deserialize, Serialize)]
pub struct LlmProxyPublicKeyPayload {
    pub pubkey_pem: String,
    pub signature_b64: String,
    pub ca_cert_pem: String,
}