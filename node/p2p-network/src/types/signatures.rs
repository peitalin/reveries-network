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
use umbral_pre::Signature as UmbralSignature;
use regex::Regex;
use alloy_primitives::{Address, Signature, B256};
use sha3::{Digest, Keccak256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignatureType {
    /// ECDSA signature (e.g. from Ethereum)
    Ecdsa(Vec<u8>),
    /// Umbral's ECDSA signature
    Umbral(Vec<u8>),
    /// ED25519 signature (e.g. from Solana or NEAR)
    Ed25519(Vec<u8>),
}

impl From<Signature> for SignatureType {
    fn from(sig: Signature) -> Self {
        SignatureType::Ecdsa(sig.as_bytes().to_vec())
    }
}

impl From<String> for SignatureType {
    fn from(sig: String) -> Self {
        let re = Regex::new(r"^(Umbral|Ecdsa|Ed25519)\((.*)\)$").unwrap();

        if let Some(captures) = re.captures(&sig) {
            let prefix = captures.get(1).unwrap().as_str();
            let content = captures.get(2).unwrap().as_str();
            let bytes = hex::decode(content).unwrap();

            match prefix {
                "Umbral" => SignatureType::Umbral(bytes),
                "Ecdsa" => SignatureType::Ecdsa(bytes),
                "Ed25519" => SignatureType::Ed25519(bytes),
                _ => unreachable!(), // We know the regex only matches these prefixes
            }
        } else {
            // Fallback to the original behavior - assume it's an Umbral signature
            let bytes = hex::decode(sig).unwrap();
            SignatureType::Umbral(bytes)
        }
    }
}

impl SignatureType {
    pub fn deserialize(&self) -> Result<UmbralSignature> {
        match self {
            SignatureType::Umbral(bytes) => {
                serde_json::from_slice::<UmbralSignature>(bytes)
            }
            SignatureType::Ecdsa(bytes) => {
                unimplemented!("Deserializing ECDSA Ethereum signatures is not meaningful in this context");
            }
            SignatureType::Ed25519(bytes) => {
                unimplemented!("Ed25519 signatures are not implemented yet");
            },
        }.map_err(|e| anyhow!("Failed to deserialize signature: {}", e))
    }

    pub fn verify_sig(&self, verifying_key: &VerifyingKey, message_hash: &[u8]) -> bool {
        match self {
            SignatureType::Umbral(bytes) => {
                if let Ok(signature) = serde_json::from_slice::<UmbralSignature>(bytes) {
                    if let VerifyingKey::Umbral(umbral_verifying_key) = verifying_key {
                        if signature.verify(&umbral_verifying_key, message_hash) {
                            return true;
                        } else {
                            tracing::warn!("Umbral signature verification failed");
                            return false;
                        }
                    } else {
                        tracing::warn!("Verifying key must be an Umbral key for Umbral signature verification");
                    }
                } else {
                   tracing::warn!("Failed to deserialize Umbral signature from bytes");
                }
                false
            },
            SignatureType::Ecdsa(sig_bytes) => {
                if let VerifyingKey::Ecdsa(expected_address) = verifying_key {
                    // Convert message hash slice to B256
                    let hash = B256::from_slice(message_hash);
                    // Parse the signature bytes into an alloy Signature using TryFrom
                    match Signature::try_from(sig_bytes.as_slice()) {
                        Ok(signature) => {
                            // Recover the address from the hash and signature
                            match signature.recover_address_from_prehash(&hash) {
                                Ok(recovered_address) => {
                                    // Compare the recovered address to the expected address
                                    recovered_address == *expected_address
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to recover address from ECDSA signature: {}", e);
                                    false
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse ECDSA signature from bytes: {}", e);
                            false
                        }
                    }
                } else {
                    tracing::warn!("Verifying key must be an Ecdsa Address for ECDSA signature verification");
                    false
                }
            },
            SignatureType::Ed25519(_) => {
                unimplemented!("Ed25519 signatures are not implemented yet");
            },
        }
    }
}

impl std::fmt::Display for SignatureType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hex_sig = match self {
            SignatureType::Umbral(bytes) => {
                format!("Umbral(0x{})", hex::encode(bytes.clone()))
            }
            SignatureType::Ecdsa(bytes) => {
                format!("ECDSA(0x{})", hex::encode(bytes.clone()))
            }
            SignatureType::Ed25519(bytes) => {
                format!("Ed25519(0x{})", hex::encode(bytes.clone()))
            }
        };
        write!(f, "{}", hex_sig)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum VerifyingKey {
    Umbral(umbral_pre::PublicKey),
    Ecdsa(Address),
    Ed25519(String),
}

impl From<Address> for VerifyingKey {
    fn from(address: Address) -> Self {
        VerifyingKey::Ecdsa(address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256};
    use alloy_signer::Signer;
    use alloy_signer_local::LocalWallet;

    async fn create_test_signer() -> Result<LocalWallet> {
        let wallet = LocalWallet::random(); // Generate a random wallet
        Ok(wallet)
    }

    #[test]
    fn test_signature_type_from_string_with_umbral_prefix() {
        let hex_data = "deadbeef";
        let sig_string = format!("Umbral({})", hex_data);

        let sig_type = SignatureType::from(sig_string);

        match sig_type {
            SignatureType::Umbral(bytes) => {
                assert_eq!(bytes, hex::decode(hex_data).unwrap());
            },
            _ => panic!("Expected Umbral signature type"),
        }
    }

    #[test]
    fn test_signature_type_from_string_with_ecdsa_prefix() {
        let hex_data = "cafebabe";
        let sig_string = format!("Ecdsa({})", hex_data);

        let sig_type = SignatureType::from(sig_string);

        match sig_type {
            SignatureType::Ecdsa(bytes) => {
                assert_eq!(bytes, hex::decode(hex_data).unwrap());
            },
            _ => panic!("Expected Ecdsa signature type"),
        }
    }

    #[test]
    fn test_signature_type_from_string_with_ed25519_prefix() {
        let hex_data = "f00dfeed";
        let sig_string = format!("Ed25519({})", hex_data);

        let sig_type = SignatureType::from(sig_string);
        println!("signature_type: {}", sig_type);

        match sig_type {
            SignatureType::Ed25519(bytes) => {
                assert_eq!(bytes, hex::decode(hex_data).unwrap());
            },
            _ => panic!("Expected Ed25519 signature type"),
        }
    }

    #[test]
    fn test_signature_type_from_string_without_prefix() {
        let hex_data = "abcdef1234";

        let sig_type = SignatureType::from(hex_data.to_string());
        println!("signature_type: {}", sig_type);

        match sig_type {
            SignatureType::Umbral(bytes) => {
                assert_eq!(bytes, hex::decode(hex_data).unwrap());
            },
            _ => panic!("Expected default Umbral signature type"),
        }
    }

    #[tokio::test]
    async fn test_ecdsa_signature_verification() -> Result<()> {
        let signer = create_test_signer().await?;
        let signer_address: Address = signer.address();

        let test_message = b"Hello, world!";
        let digest = Keccak256::digest(test_message);
        let hash = B256::from_slice(&digest);

        let signature = signer.sign_hash(&hash).await?;
        // Convert signature to standard R, S, V bytes
        let signature_bytes = signature.as_bytes().to_vec();

        let signature_type = SignatureType::Ecdsa(signature_bytes);
        println!("signature_type: {}", signature_type);

        let verifying_key = VerifyingKey::Ecdsa(signer_address);

        assert!(signature_type.verify_sig(&verifying_key, &digest));

        let invalid_message = b"Invalid message";
        let invalid_digest = Keccak256::digest(invalid_message);
        assert!(!signature_type.verify_sig(&verifying_key, &invalid_digest));

        let wrong_address_str = "0x1111111111111111111111111111111111111111";
        let wrong_address: Address = wrong_address_str.parse()?;
        let wrong_key = VerifyingKey::Ecdsa(wrong_address);
        assert!(!signature_type.verify_sig(&wrong_key, &digest));

        Ok(())
    }
}



