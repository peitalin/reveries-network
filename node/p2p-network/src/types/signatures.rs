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
use std::str::FromStr;
use alloy_primitives::hex;

type SignatureBytes = Vec<u8>;
type ContractMethod = String;
type ContractArgs = serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessKey {
    /// Requires valid ECDSA signature (e.g. from Ethereum)
    EcdsaSignature(SignatureBytes),
    /// Requires valid Umbral's ECDSA signature
    UmbralSignature(SignatureBytes),
    /// Requires valid ED25519 signature (e.g. from Solana or NEAR)
    Ed25519Signature(SignatureBytes),
    /// NEAR Contract Access Condition
    NearContract(
        String, // contract address
        String, // spender address
        u128 // amount
    ),
    /// Ethereum Contract Access Condition
    EthContract(
        alloy_primitives::Address, // contract address
        ContractMethod, // contract method name
        ContractArgs // contract calldata
    ),
}

impl From<Signature> for AccessKey {
    fn from(sig: Signature) -> Self {
        AccessKey::EcdsaSignature(sig.as_bytes().to_vec())
    }
}

impl From<String> for AccessKey {
    fn from(sig: String) -> Self {
        let re = Regex::new(r"^(Umbral|Ecdsa|Ed25519)\((.*)\)$").unwrap();

        if let Some(captures) = re.captures(&sig) {
            let prefix = captures.get(1).unwrap().as_str();
            let content = captures.get(2).unwrap().as_str();
            let bytes = hex::decode(content).unwrap();

            match prefix {
                "Umbral" => AccessKey::UmbralSignature(bytes),
                "Ecdsa" => AccessKey::EcdsaSignature(bytes),
                "Ed25519" => AccessKey::Ed25519Signature(bytes),
                _ => unreachable!(), // We know the regex only matches these prefixes
            }
        } else {
            // Fallback to the original behavior - assume it's an Umbral signature
            let bytes = hex::decode(sig).unwrap();
            AccessKey::UmbralSignature(bytes)
        }
    }
}

impl AccessKey {
    pub fn deserialize(&self) -> Result<UmbralSignature> {
        match self {
            AccessKey::UmbralSignature(sig_bytes) => {
                serde_json::from_slice::<UmbralSignature>(sig_bytes)
            }
            AccessKey::EcdsaSignature(sig_bytes) => {
                unimplemented!("Deserializing ECDSA Ethereum signatures is not meaningful in this context");
            }
            AccessKey::Ed25519Signature(sig_bytes) => {
                unimplemented!("Ed25519 signatures are not implemented yet");
            },
            AccessKey::NearContract(
                address,
                method_name,
                calldata
            ) => {
                unimplemented!("NearContract access keys are not implemented yet");
            },
            AccessKey::EthContract(
                address,
                method_name,
                calldata
            ) => {
                unimplemented!("EthContract access keys are not implemented yet");
            },
        }.map_err(|e| anyhow!("Failed to deserialize signature: {}", e))
    }

    pub fn verify_access<T: ToString>(&self, access_condition: &AccessCondition, reverie_id: T) -> bool {
        let digest = Keccak256::digest(reverie_id.to_string().as_bytes());
        let message_hash = digest.as_slice();
        match self {
            AccessKey::UmbralSignature(sig_bytes) => {
                if let AccessCondition::Umbral(expected_address) = access_condition {
                    if let Ok(signature) = serde_json::from_slice::<UmbralSignature>(sig_bytes) {
                        if signature.verify(&expected_address, message_hash) {
                            return true;
                        } else {
                            tracing::warn!("Umbral signature verification failed");
                        }
                    } else {
                        tracing::warn!("Failed to deserialize Umbral signature from bytes");
                    }
                } else {
                    tracing::warn!("AccessKey::UmbralSignature cannot be used for AccessCondition::{}", access_condition.get_type());
                }
                false
            },
            AccessKey::EcdsaSignature(sig_bytes) => {
                if let AccessCondition::Ecdsa(expected_address) = access_condition {
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
            AccessKey::Ed25519Signature(_) => {
                unimplemented!("Ed25519 signatures are not implemented yet");
            },
            AccessKey::NearContract(
                contract_address,
                contract_method_name,
                contract_args
            ) => {
                unimplemented!("NearContract access keys are not implemented yet");
            },
            AccessKey::EthContract(
                contract_address,
                contract_method_name,
                contract_args
            ) => {
                unimplemented!("EthContract access keys are not implemented yet");
            },
        }
    }
}

pub fn create_digest_hash(reverie_id: &ReverieId, nonce: usize, timestamp: usize) -> B256 {
    let digest = Keccak256::digest(reverie_id.as_bytes());
    let hash = B256::from_slice(digest.as_slice());
    hash
}

impl std::fmt::Display for AccessKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hex_sig = match self {
            AccessKey::UmbralSignature(sig_bytes) => {
                format!("UmbralSignature(0x{})", hex::encode(sig_bytes.clone()))
            }
            AccessKey::EcdsaSignature(sig_bytes) => {
                format!("ECDSASignature(0x{})", hex::encode(sig_bytes.clone()))
            }
            AccessKey::Ed25519Signature(sig_bytes) => {
                format!("Ed25519Signature(0x{})", hex::encode(sig_bytes.clone()))
            }
            AccessKey::NearContract(
                address,
                method_name,
                calldata
            ) => {
                format!("NearContract({}, {}, {})", address, method_name, calldata)
            }
            AccessKey::EthContract(
                address,
                method_name,
                calldata
            ) => {
                format!("EthContract(0x{}, {}, {})", address, method_name, calldata)
            }
        };
        write!(f, "{}", hex_sig)
    }
}

impl AccessKey {
    pub fn get_type(&self) -> String {
        match self {
            AccessKey::UmbralSignature(_) => "umbral".to_string(),
            AccessKey::EcdsaSignature(_) => "ecdsa".to_string(),
            AccessKey::Ed25519Signature(_) => "ed25519".to_string(),
            AccessKey::NearContract(_, _, _) => "near_contract".to_string(),
            AccessKey::EthContract(_, _, _) => "eth_contract".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum AccessCondition {
    /// Requires a signature matching the Umbral public key
    Umbral(umbral_pre::PublicKey),
    /// Requires a signature matching the ECDSA address
    Ecdsa(Address),
    /// Requires a signature matching the ED25519 address
    Ed25519(String),
    /// Requires NEAR contract evaluation (preconditions)
    NearContract(
        // contract address
        near_primitives::types::AccountId,
        // spender address
        near_primitives::types::AccountId,
        // amount
        u128
    ),
    /// Requires ETH contract evaluation (preconditions)
    EthContract(
        // contract address
        alloy_primitives::Address,
        // access function name
        String,
        // access function args
        ContractArgs
    ),
}


impl std::fmt::Display for AccessCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hex_sig = match self {
            AccessCondition::Umbral(pubkey) => {
                format!("umbral:{}", pubkey.to_string())
            }
            AccessCondition::Ecdsa(pubkey) => {
                format!("ecdsa:{}", pubkey.to_string())
            }
            AccessCondition::Ed25519(pubkey) => {
                format!("ed25519:{}", pubkey.to_string())
            }
            AccessCondition::NearContract(
                address,
                access_function_name,
                access_function_args
            ) => {
                format!("near_contract:{}:{}:{}", address.to_string(), access_function_name, access_function_args)
            }
            AccessCondition::EthContract(
                address,
                access_function_name,
                access_function_args
            ) => {
                format!("eth_contract:{}:{}:{}", address.to_string(), access_function_name, access_function_args)
            }
        };
        write!(f, "{}", hex_sig)
    }
}

impl AccessCondition {
    pub fn get_type(&self) -> String {
        match self {
            AccessCondition::Umbral(_) => "Umbral".to_string(),
            AccessCondition::Ecdsa(_) => "Ecdsa".to_string(),
            AccessCondition::Ed25519(_) => "Ed25519".to_string(),
            AccessCondition::NearContract(_, _, _) => "NearContract".to_string(),
            AccessCondition::EthContract(_, _, _) => "EthContract".to_string(),
        }
    }
}

impl From<Address> for AccessCondition {
    fn from(address: Address) -> Self {
        AccessCondition::Ecdsa(address)
    }
}

impl FromStr for AccessCondition {
    type Err = color_eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((prefix, value)) = s.split_once(':') {
            match prefix {
                "ecdsa" => {
                    let addr = Address::from_str(value)
                        .map_err(|e| anyhow!("Failed to parse ECDSA address from '{}': {}", value, e))?;
                    Ok(AccessCondition::Ecdsa(addr))
                }
                "umbral" => {
                    // Assuming PublicKey can be deserialized from hex string
                    let bytes = hex::decode(value).map_err(|e| anyhow!("Invalid hex for Umbral key '{}': {}", value, e))?;
                    let pk = umbral_pre::PublicKey::try_from_compressed_bytes(&bytes[..])
                        .map_err(|e| anyhow!("Failed to deserialize Umbral PublicKey from bytes: {}", e))?;
                     Ok(AccessCondition::Umbral(pk))
                }
                "ed25519" => {
                    Ok(AccessCondition::Ed25519(value.to_string()))
                }
                "contract" => {
                    // TODO: implement contract FromStr for AccessCondition
                    unimplemented!("contract serialization for AccessCondition not implemented yet");
                    // let args= value.split(':').collect::<Vec<&str>>();
                    // if args.len() != 3 {
                    //     return Err(anyhow!("Invalid AccessCondition format: expected 'contract:address:calldata_hex', got '{}'", value));
                    // }
                    // let addr = Address::from_str(args[0]).map_err(|e| anyhow!("Invalid contract address '{}': {}", args[0], e))?;
                    // let access_function_name = String::from(args[1]);
                    // let access_function_args = serde_json::to_value(args[2]).map_err(|e| anyhow!("Invalid hex for access_function_args '{}': {}", args[2], e))?;
                    // Ok(AccessCondition::Contract(addr, access_function_name, access_function_args))
                }
                _ => Err(anyhow!("Unknown AccessCondition prefix: '{}'", prefix)),
            }
        } else {
            Err(anyhow!("Invalid AccessCondition format: expected 'prefix:value', got '{}'", s))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256};
    use alloy_signer::Signer;
    use alloy_signer_local::PrivateKeySigner;

    async fn create_test_signer() -> Result<PrivateKeySigner> {
        // Generate a random wallet
        let wallet = PrivateKeySigner::random();
        Ok(wallet)
    }

    #[test]
    fn test_signature_type_from_string_with_umbral_prefix() {
        let hex_data = "deadbeef";
        let sig_string = format!("Umbral({})", hex_data);

        let sig_type = AccessKey::from(sig_string);

        match sig_type {
            AccessKey::UmbralSignature(bytes) => {
                assert_eq!(bytes, hex::decode(hex_data).unwrap());
            },
            _ => panic!("Expected Umbral signature type"),
        }
    }

    #[test]
    fn test_signature_type_from_string_with_ecdsa_prefix() {
        let hex_data = "cafebabe";
        let sig_string = format!("Ecdsa({})", hex_data);

        let sig_type = AccessKey::from(sig_string);

        match sig_type {
            AccessKey::EcdsaSignature(bytes) => {
                assert_eq!(bytes, hex::decode(hex_data).unwrap());
            },
            _ => panic!("Expected Ecdsa signature type"),
        }
    }

    #[test]
    fn test_signature_type_from_string_with_ed25519_prefix() {
        let hex_data = "f00dfeed";
        let sig_string = format!("Ed25519({})", hex_data);

        let sig_type = AccessKey::from(sig_string);
        println!("signature_type: {}", sig_type);

        match sig_type {
            AccessKey::Ed25519Signature(bytes) => {
                assert_eq!(bytes, hex::decode(hex_data).unwrap());
            },
            _ => panic!("Expected Ed25519 signature type"),
        }
    }

    #[test]
    fn test_signature_type_from_string_without_prefix() {
        let hex_data = "abcdef1234";

        let sig_type = AccessKey::from(hex_data.to_string());
        println!("signature_type: {}", sig_type);

        match sig_type {
            AccessKey::UmbralSignature(bytes) => {
                assert_eq!(bytes, hex::decode(hex_data).unwrap());
            },
            _ => panic!("Expected default Umbral signature type"),
        }
    }

    #[tokio::test]
    async fn test_ecdsa_signature_verification() -> Result<()> {
        let signer = create_test_signer().await?;
        let signer_address: Address = signer.address();

        let test_message = "Hello, world!";
        let digest = Keccak256::digest(test_message.as_bytes());
        let hash = B256::from_slice(&digest);

        let signature = signer.sign_hash(&hash).await?;
        // Convert signature to standard R, S, V bytes
        let signature_bytes = signature.as_bytes().to_vec();

        let signature_type = AccessKey::EcdsaSignature(signature_bytes);
        println!("signature_type: {}", signature_type);

        let verifying_key = AccessCondition::Ecdsa(signer_address);

        assert!(signature_type.verify_access(&verifying_key, &test_message));

        let invalid_message = "Invalid message";
        let invalid_digest = Keccak256::digest(invalid_message.as_bytes());
        assert!(!signature_type.verify_access(&verifying_key, &invalid_message));

        let wrong_address_str = "0x1111111111111111111111111111111111111111";
        let wrong_address: Address = wrong_address_str.parse()?;
        let wrong_key = AccessCondition::Ecdsa(wrong_address);
        assert!(!signature_type.verify_access(&wrong_key, &test_message));

        Ok(())
    }
}



