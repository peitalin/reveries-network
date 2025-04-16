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
use regex::Regex;
use ethers::types::{Address, Signature as EthersSignature};
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
    pub fn deserialize(&self) -> Result<umbral_pre::Signature> {
        match self {
            SignatureType::Umbral(bytes) => {
                serde_json::from_slice::<umbral_pre::Signature>(bytes)
            }
            SignatureType::Ecdsa(bytes) => {
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
            SignatureType::Ecdsa(bytes) => {
                // We expect the bytes to be a serialized Ethereum signature
                if let Ok(eth_signature) = EthersSignature::try_from(bytes.as_slice()) {
                    if let VerifyingKey::Ecdsa(eth_address) = verifying_key {
                        // Parse the Ethereum address from the string
                        if let Ok(address) = eth_address.parse::<Address>() {
                            // The message is already a hash in the test case
                            let message_hash = ethers::types::H256::from_slice(message);

                            // Recover the signer's address from the signature and message hash
                            if let Ok(recovered_address) = eth_signature.recover(message_hash) {
                                // Check if the recovered address matches the expected address
                                return recovered_address == address;
                            } else {
                                tracing::warn!("Failed to recover address from ECDSA signature");
                            }
                        } else {
                            tracing::warn!("Invalid Ethereum address format");
                        }
                    } else {
                        tracing::warn!("Verifying key must be an Ethereum address for ECDSA signature verification");
                    }
                } else {
                    tracing::warn!("Failed to parse ECDSA signature from bytes");
                }
                false
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
    Ecdsa(String), // Ethereum address as a hex string
    Ed25519(String), // TODO: update this
}

impl From<ethers::types::Address> for VerifyingKey {
    fn from(address: ethers::types::Address) -> Self {
        VerifyingKey::Ecdsa(format!("{:?}", address))
    }
}

// // Create a test that
// - creates a Ed25519 Keypair
// - creates a digestHash from a message: e.g. keccak256("Hello, world!")
// - signs the digestHash using the Ethereum Keypair
// - verifies the signature using the Ethereum public key
// - prints the signature and the message


#[cfg(test)]
mod tests {
    use super::*;
    use ethers::signers::Signer;

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
        // Create a signer using the ethers library
        let signer = create_test_signer().await?;
        let signer_address = signer.address();

        // Create a test message and hash it
        let test_message = b"Hello, world!";
        let digest = Keccak256::digest(test_message);
        let h256_digest = ethers::types::H256::from_slice(&digest);

        // Sign the digest hash
        let signature = signer.sign_hash(h256_digest)?;
        let signature_bytes = signature.to_vec();

        // Create a SignatureType using the signature
        let signature_type = SignatureType::Ecdsa(signature_bytes);
        println!("signature_type: {}", signature_type);

        // Create a VerifyingKey from the signer's address
        let verifying_key = VerifyingKey::Ecdsa(format!("{:?}", signer_address));

        // Verify the signature - should return true for valid signature
        assert!(signature_type.verify_sig(&verifying_key, &digest));

        // Test with modified message (should fail verification)
        let invalid_message = b"Invalid message";
        let invalid_digest = Keccak256::digest(invalid_message);
        assert!(!signature_type.verify_sig(&verifying_key, &invalid_digest));

        // Test with wrong verifying key (should fail verification)
        let wrong_address = "0x0000000000000000000000000000000000000001";
        let wrong_key = VerifyingKey::Ecdsa(wrong_address.to_string());
        assert!(!signature_type.verify_sig(&wrong_key, &digest));

        Ok(())
    }

    async fn create_test_signer() -> Result<ethers::signers::LocalWallet> {
        use ethers::core::utils::Anvil;
        use ethers::signers::LocalWallet;

        let anvil = Anvil::new().spawn();
        let wallet: LocalWallet = anvil.keys()[0].clone().into();
        Ok(wallet)
    }
}



