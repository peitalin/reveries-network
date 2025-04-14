use color_eyre::{Result, eyre::anyhow};
use libp2p::PeerId;
use crate::types::{
    ReverieKeyfragMessage,
    ReverieMessage,
    ReverieId,
    ReverieNameWithNonce
};
use serde::{Deserialize, Serialize};
use serde_json;
use umbral_pre::Signature;



#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignatureType {
    /// ECDSA signature (e.g. from Ethereum)
    Ethereum(Vec<u8>),
    /// Umbral's ECDSA signature
    Umbral(Vec<u8>),
    /// ED25519 signature (e.g. from Solana or NEAR)
    Ed25519(Vec<u8>),
}

impl SignatureType {
    pub fn deserialize(&self) -> Result<umbral_pre::Signature> {
        match self {
            SignatureType::Umbral(bytes) => {
                serde_json::from_slice::<umbral_pre::Signature>(bytes)
            }
            SignatureType::Ethereum(bytes) => {
                unimplemented!("ECDSA Ethereum signatures are not implemented yet");
                // serde_json::from_slice::<umbral_pre::Signature>(bytes)
            }
            SignatureType::Ed25519(bytes) => {
                unimplemented!("Ed25519 signatures are not implemented yet");
            },
        }.map_err(|e| anyhow!("Failed to deserialize signature: {}", e))
    }

    pub fn verify_sig(&self, verifying_key: &VerifyingKey, message: &[u8]) -> bool {
        match self {
            SignatureType::Umbral(bytes) => {
                // Deserialize the signature from bytes
                if let Ok(signature) = serde_json::from_slice::<umbral_pre::Signature>(bytes) {
                    if let VerifyingKey::Umbral(umbral_verifying_key) = verifying_key {
                        // Verify the signature using Umbral's verify method
                        if signature.verify(&umbral_verifying_key, message) {
                            return true;
                        } else {
                            tracing::warn!("Signature verification failed");
                            return false;
                        }
                    } else {
                        unimplemented!("Verifying key type must be an Umbral verifying key");
                    }
                }
                false
            },
            // For other signature types, verification would need to be implemented
            // For now, return false for non-Umbral signatures
            SignatureType::Ethereum(bytes) => {
                unimplemented!("ECDSA Ethereum signatures are not implemented yet");
            },
            SignatureType::Ed25519(bytes) => {
                unimplemented!("Ed25519 signatures are not implemented yet");
            },
        }
    }
}

impl std::fmt::Display for SignatureType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hex_sig = match self {
            SignatureType::Umbral(bytes) => {
                format!("Umbral({})", hex::encode(bytes.clone()))
            }
            SignatureType::Ethereum(bytes) => {
                format!("Ethereum({})", hex::encode(bytes.clone()))
            }
            SignatureType::Ed25519(bytes) => {
                format!("Ed25519({})", hex::encode(bytes.clone()))
            }
        };
        write!(f, "SignatureType::{}", hex_sig)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum VerifyingKey {
    Umbral(umbral_pre::PublicKey),
    Ethereum(String), // TODO: update this
    Ed25519(String), // TODO: update this
}
