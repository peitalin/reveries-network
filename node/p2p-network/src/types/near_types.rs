use alloy_primitives::Address;
use color_eyre::{eyre::anyhow, Result};
use std::str::FromStr;
use runtime::near_runtime::{
    AccessCondition as NearRuntimeAccessCondition,
};
use crate::types::{
    AccessCondition as P2PNetworkAccessCondition,
};


impl TryFrom<&P2PNetworkAccessCondition> for NearRuntimeAccessCondition {
    type Error = color_eyre::Report;

    fn try_from(p2p_ac: &P2PNetworkAccessCondition) -> Result<Self, Self::Error> {
        match p2p_ac {
            P2PNetworkAccessCondition::Umbral(pk) => {
                Ok(NearRuntimeAccessCondition::Umbral(hex::encode(pk.to_compressed_bytes())))
            }
            P2PNetworkAccessCondition::Ecdsa(addr) => {
                Ok(NearRuntimeAccessCondition::Ecdsa(addr.to_string()))
            }
            P2PNetworkAccessCondition::Ed25519(pk_str) => {
                Ok(NearRuntimeAccessCondition::Ed25519(pk_str.clone()))
            }
            P2PNetworkAccessCondition::NearContract(contract_account_id, user_account_id, amount) => {
                Ok(NearRuntimeAccessCondition::Contract {
                    contract_account_id: contract_account_id.to_string(),
                    user_account_id: user_account_id.to_string(),
                    amount: amount.clone(),

                })
            }
            P2PNetworkAccessCondition::EthContract(addr, name, args) => {
                unimplemented!("EthContract access conditions are not implemented yet");
            }
        }
    }
}

impl TryFrom<&NearRuntimeAccessCondition> for P2PNetworkAccessCondition {
    type Error = color_eyre::Report;

    fn try_from(near_ac: &NearRuntimeAccessCondition) -> Result<Self, Self::Error> {
        match near_ac {
            NearRuntimeAccessCondition::Umbral(hex_pk_str) => {
                let bytes = hex::decode(hex_pk_str)
                    .map_err(|e| anyhow!("Invalid hex for Umbral key string '{}': {}", hex_pk_str, e))?;
                let pk = umbral_pre::PublicKey::try_from_compressed_bytes(&bytes)
                    .map_err(|e| anyhow!("Failed to deserialize Umbral PublicKey from bytes: {}", e))?;
                Ok(P2PNetworkAccessCondition::Umbral(pk))
            }
            NearRuntimeAccessCondition::Ecdsa(addr_str) => {
                let addr = Address::from_str(addr_str)
                    .map_err(|e| anyhow!("Failed to parse ECDSA address from string '{}': {}", addr_str, e))?;
                Ok(P2PNetworkAccessCondition::Ecdsa(addr))
            }
            NearRuntimeAccessCondition::Ed25519(pk_str) => {
                Ok(P2PNetworkAccessCondition::Ed25519(pk_str.clone()))
            }
            NearRuntimeAccessCondition::Contract {
                contract_account_id,
                user_account_id,
                amount
            } => {
                let contract_account = near_primitives::types::AccountId::from_str(contract_account_id)
                    .map_err(|e| anyhow!("Failed to parse contract address from string '{}': {}", contract_account_id, e))?;
                let user_account = near_primitives::types::AccountId::from_str(user_account_id)
                    .map_err(|e| anyhow!("Failed to parse user account id from string '{}': {}", user_account_id, e))?;
                Ok(P2PNetworkAccessCondition::NearContract(contract_account, user_account, *amount))
            }
        }
    }
}