use color_eyre::Result;
use tracing::{info, warn, error};
use std::str::FromStr; // For parsing strings to addresses etc.

// NEAR imports
use near_jsonrpc_client::{methods, JsonRpcClient};
use near_jsonrpc_primitives::types::query::QueryResponseKind;
use near_primitives::{
    transaction::{SignedTransaction, Transaction, Action, FunctionCallAction},
    types::{
        AccountId,
        BlockReference,
        Finality,
        Balance,
        FunctionArgs,
        Gas,
        Nonce
    },
    views::{
        QueryRequest,
        FinalExecutionOutcomeView,
        CallResult,
        FinalExecutionStatus,
        AccessKeyView
    },
    hash::CryptoHash,
};
use near_crypto::{InMemorySigner, KeyType, SecretKey, Signature, Signer};
use borsh::BorshSerialize; // For serializing the transaction for hashing
// Temporarily comment out omni_transaction to resolve potential name collisions
use omni_transaction;
use omni_transaction::near::NearTransaction;
use omni_transaction::near::{NearTransactionBuilder};
use omni_transaction::near::types::{
    Action as OmniAction,
    FunctionCallAction as OmniFunctionCallAction,
    PublicKey as OmniPublicKey,
    ED25519PublicKey as OmniED25519Data,
    Secp256K1PublicKey as OmniSecp256k1Data,
    BlockHash as OmniBlockHash,
    U64 as OmniU64,
    U128 as OmniU128,
};
use omni_transaction::TxBuilder;


#[derive(Clone, Debug)]
pub struct NearConfig {
    pub near_rpc_url: String,
}

impl Default for NearConfig {
    fn default() -> Self {
        Self {
            near_rpc_url: "https://rpc.testnet.near.org".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct NearRuntime {
    near_client: JsonRpcClient,
    config: NearConfig,
}

impl NearRuntime {
    pub async fn new(config: NearConfig) -> Result<Self> {
        info!("NEAR RPC URL: {}", config.near_rpc_url);
        let near_client = JsonRpcClient::connect(&config.near_rpc_url);

        Ok(Self {
            near_client,
            config,
        })
    }

    pub async fn get_near_account_balance(&self, account_id_str: &str) -> Result<Balance> {
        let account_id = AccountId::from_str(account_id_str)?;
        info!("Fetching NEAR balance for account: {}", account_id);

        let request_payload = methods::query::RpcQueryRequest {
            block_reference: BlockReference::Finality(Finality::Final),
            request: QueryRequest::ViewAccount { account_id },
        };

        let response = self.near_client.call(request_payload).await
            .map_err(|e| color_eyre::eyre::eyre!("RPC call failed for get_near_account_balance: {}", e))?;

        match response.kind {
            QueryResponseKind::ViewAccount(view) => {
                println!("{:#?}", view);
                Ok(view.amount)
            },
            _ => {
                Err(color_eyre::eyre::eyre!("Unexpected query response kind for account balance. Got: {:?}", response.kind))
            }
        }
    }

    pub async fn send_near_transaction(&self, signed_transaction: SignedTransaction) -> Result<FinalExecutionOutcomeView> {
        info!("Sending NEAR transaction...");
        let request = methods::broadcast_tx_commit::RpcBroadcastTxCommitRequest {
            signed_transaction,
        };
        let outcome = self.near_client.call(request).await
            .map_err(|e| color_eyre::eyre::eyre!("RPC call failed for send_near_transaction: {}", e))?;
        Ok(outcome)
    }

    pub async fn read_near_contract_state(
        &self,
        contract_id_str: &str,
        method_name: String,
        args: FunctionArgs
    ) -> Result<CallResult> {

        let contract_id = AccountId::from_str(contract_id_str)?;
        info!("Reading NEAR contract state: {} method {} with args: {:?}", contract_id, method_name, args.clone());

        let query_request_payload = methods::query::RpcQueryRequest {
            block_reference: BlockReference::Finality(Finality::Final),
            request: QueryRequest::CallFunction {
                account_id: contract_id,
                method_name,
                args,
            },
        };

        let response = self.near_client.call(query_request_payload).await
            .map_err(|e| color_eyre::eyre::eyre!("RPC call failed for read_near_contract_state: {}", e))?;

        match response.kind {
            QueryResponseKind::CallResult(result) => Ok(result),
            _ => Err(color_eyre::eyre::eyre!("Unexpected query response kind for contract call. Got: {:?}", response.kind)),
        }
    }
}


pub async fn near_example() -> Result<()> {
    let config = NearConfig::default();
    let runtime = NearRuntime::new(config).await?;

    let near_account_str = "yellow-loong.testnet";
    let balance = runtime.get_near_account_balance(near_account_str).await?;
    info!("NEAR balance for {}: {}", near_account_str, balance);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*; // Import items from the parent module (NearRuntime, NearConfig, etc.)
    use color_eyre::eyre::eyre; // For easy error creation in tests
    use dotenv::dotenv;

    use near_primitives::transaction::{Transaction, TransactionV0};

    fn setup_test_logger() {
        let _ = tracing_subscriber::fmt().with_env_filter("info").try_init();
    }

    #[tokio::test]
    async fn test_near_runtime_creation() -> Result<()> {
        setup_test_logger();
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config).await;
        assert!(runtime.is_ok(), "Failed to create NearRuntime: {:?}", runtime.err());
        Ok(())
    }

    #[tokio::test]
    async fn test_get_known_account_balance() -> Result<()> {
        setup_test_logger();
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config).await?;

        let account_id = "yellow-loong.testnet"; // A known account with a likely stable balance
        let balance_result = runtime.get_near_account_balance(account_id).await;

        assert!(balance_result.is_ok(), "Failed to get balance for {}: {:?}", account_id, balance_result.err());
        let balance = balance_result.unwrap();
        info!("Balance for {}: {}", account_id, balance);
        assert!(balance > 0, "Balance for {} should be greater than 0", account_id);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_non_existent_account_balance() -> Result<()> {
        setup_test_logger();
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config).await?;

        // A clearly non-existent account ID format, or a randomly generated one unlikely to exist
        let account_id = "thisaccountassuredlydoesnotexist.testnet";
        let balance_result = runtime.get_near_account_balance(account_id).await;

        assert!(balance_result.is_err(), "Getting balance for non-existent account {} should fail", account_id);
        info!("Correctly failed to get balance for non-existent account {}: {:?}", account_id, balance_result.err().unwrap());
        Ok(())
    }

    #[tokio::test]
    async fn test_read_contract_state_fails_for_no_code() -> Result<()> {
        setup_test_logger();
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config).await?;

        let contract_id = "contract.testnet"; // This account has no code
        let method_name = "get_status".to_string();
        // Arguments don't really matter if the contract doesn't exist
        let args_json = serde_json::json!({ "account_id": "yellow-loong.testnet" });
        let args = FunctionArgs::from(args_json.to_string().into_bytes());

        let state_result = runtime.read_near_contract_state(contract_id, method_name, args).await;

        // Expect an error because the contract (code) does not exist
        assert!(state_result.is_err(), "Call to contract with no code should fail.");
        let error_message = state_result.err().unwrap().to_string();
        info!("Correctly received error for no code: {}", error_message);
        assert!(error_message.contains("CodeDoesNotExist") || error_message.contains("wasm execution failed"));

        Ok(())
    }

    // NOTE: requires a real contract deployed on testnet with the right methods
    #[tokio::test]
    async fn test_read_existing_contract_state() -> Result<()> {
        setup_test_logger();
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config).await?;

        let contract_id = "yellow-loong.testnet"; // Find a real contract
        let method_name = "get_greeting".to_string(); // Find a real view method
        let args_json = serde_json::json!({}); // Adjust args as needed
        let args = FunctionArgs::from(args_json.to_string().into_bytes());

        let state_result = runtime.read_near_contract_state(contract_id, method_name, args).await;

        assert!(state_result.is_ok(), "Failed to read contract state: {:?}", state_result.err());
        let call_result = state_result.unwrap();
        assert!(!call_result.result.is_empty(), "Contract state result should not be empty");
        info!("Raw result from {}.{}: {:?}", contract_id, "get_greeting", String::from_utf8_lossy(&call_result.result));

        Ok(())
    }

    #[tokio::test]
    async fn test_send_transaction_example() -> Result<()> {
        dotenv().ok();
        setup_test_logger();
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config).await?;

        let signer_account_id_str = "yellow-loong.testnet";
        let signer_secret_key_str = std::env::var("NEAR_SIGNER_PRIVATE_KEY")
            .map_err(|e| eyre!("NEAR_SIGNER_PRIVATE_KEY env var not found: {}", e))?;

        let signer_account_id = AccountId::from_str(signer_account_id_str)?;
        let signer_secret_key = SecretKey::from_str(&signer_secret_key_str)?;
        let signer = InMemorySigner::from_secret_key(signer_account_id.clone(), signer_secret_key.clone());

        let receiver_id_str = "yellow-loong.testnet";
        let receiver_id = AccountId::from_str(receiver_id_str)?;
        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "set_greeting".to_string(),
            args: serde_json::json!({
                "greeting": "like tears in rain"
            }).to_string().into_bytes(),
            gas: 100_000_000_000_000,
            deposit: 0,
        }));

        let nonce_response = runtime.near_client.call(methods::query::RpcQueryRequest {
            block_reference: BlockReference::latest(),
            request: QueryRequest::ViewAccessKey {
                account_id: signer_account_id.clone(),
                public_key: signer.public_key()
            }
        }).await?;

        let current_nonce = match nonce_response.kind {
            QueryResponseKind::AccessKey(view) => view.nonce,
            _ => return Err(eyre!("Failed to get nonce, unexpected response kind: {:?}", nonce_response.kind))
        };

        let block_hash_response = runtime.near_client.call(
            methods::block::RpcBlockRequest {
                block_reference: BlockReference::latest()
            }
        ).await?;

        let block_hash = block_hash_response.header.hash;

        let transaction = Transaction::V0(TransactionV0 {
            signer_id: signer.get_account_id(),
            public_key: signer.public_key(),
            nonce: current_nonce + 1,
            receiver_id: receiver_id,
            block_hash: block_hash,
            actions: vec![action],
        });
        let tx_hash = transaction.get_hash_and_size().0;

        let signature = signer.sign(tx_hash.as_ref());
        let signed_transaction = near_primitives::transaction::SignedTransaction::new(signature, transaction.clone());

        info!("Attempting to send transaction: {:?}", signed_transaction);
        let outcome_result = runtime.send_near_transaction(signed_transaction).await;

        assert!(outcome_result.is_ok(), "send_near_transaction failed: {:?}", outcome_result.err());
        let outcome = outcome_result.unwrap();
        info!("Transaction outcome: {:?}", outcome);
        match outcome.status {
            FinalExecutionStatus::SuccessValue(value) => {
                info!("Transaction successful, return value (if any): {:?}", String::from_utf8_lossy(&value));
            }
            _ => {
                panic!("Transaction was not successful: {:?}", outcome.status);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_send_omni_transaction_example() -> Result<()> {
        dotenv().ok();
        setup_test_logger();
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config).await?;

        let signer_account_id_str = "yellow-loong.testnet";
        let signer_secret_key_str = std::env::var("NEAR_SIGNER_PRIVATE_KEY")
            .map_err(|e| eyre!("NEAR_SIGNER_PRIVATE_KEY env var not found: {}", e))?;

        let signer_account_id_primitive = AccountId::from_str(signer_account_id_str)?;
        let signer_secret_key = SecretKey::from_str(&signer_secret_key_str)?;
        let signer = InMemorySigner::from_secret_key(signer_account_id_primitive.clone(), signer_secret_key.clone());

        let receiver_id_str = "yellow-loong.testnet";
        // No need to parse receiver_id to near_primitives::types::AccountId yet, builder takes string

        // 1. Fetch nonce
        let nonce_response = runtime.near_client.call(methods::query::RpcQueryRequest {
            block_reference: BlockReference::latest(),
            request: QueryRequest::ViewAccessKey {
                account_id: signer_account_id_primitive.clone(),
                public_key: signer.public_key()
            }
        }).await?;
        let current_nonce = match nonce_response.kind {
            QueryResponseKind::AccessKey(view) => view.nonce,
            _ => return Err(eyre!("Failed to get nonce, unexpected response kind: {:?}", nonce_response.kind))
        };
        let next_nonce = current_nonce + 1;

        // 2. Fetch block hash
        let block_hash_response = runtime.near_client.call(
            methods::block::RpcBlockRequest {
                block_reference: BlockReference::latest()
            }
        ).await?;
        let near_block_hash_primitive = block_hash_response.header.hash; // This is near_primitives::hash::CryptoHash

        // 3. Convert types for Omni Builder
        let omni_signer_public_key = match signer.public_key() {
            near_crypto::PublicKey::ED25519(data) => OmniPublicKey::ED25519(OmniED25519Data(data.0)),
            near_crypto::PublicKey::SECP256K1(data) => OmniPublicKey::SECP256K1(OmniSecp256k1Data(data.as_ref().try_into().unwrap())),
        };
        let omni_block_hash = OmniBlockHash(near_block_hash_primitive.0);

        // 4. Define Action
        let omni_action = OmniAction::FunctionCall(Box::new(OmniFunctionCallAction {
            method_name: "set_greeting".to_string(),
            args: serde_json::json!({
                "greeting": "hail omni"
            }).to_string().into_bytes(),
            gas: OmniU64(100_000_000_000_000),
            deposit: OmniU128(0),
        }));

        // 5. Build Omni Transaction
        let omni_tx = NearTransactionBuilder::new()
            .signer_id(signer_account_id_str.to_string())
            .signer_public_key(omni_signer_public_key)
            .nonce(next_nonce)
            .receiver_id(receiver_id_str.to_string())
            .block_hash(omni_block_hash)
            .actions(vec![omni_action.clone()])
            .build();

        // 6. Convert Omni Transaction to Near Primitives Transaction
        let primitive_signer_id = omni_tx.signer_id.to_string().parse::<near_primitives::types::AccountId>()?;
        let primitive_receiver_id = omni_tx.receiver_id.to_string().parse::<near_primitives::types::AccountId>()?;

        let primitive_public_key = match omni_tx.signer_public_key {
            OmniPublicKey::ED25519(omni_pk_data) => near_crypto::PublicKey::ED25519(near_crypto::ED25519PublicKey(omni_pk_data.0)), // Assuming omni_pk_data is OmniED25519Data(pub [u8;32])
            OmniPublicKey::SECP256K1(omni_pk_data) => {
                // This test is set up with an ED25519 key.
                // For SECP256K1, assuming omni_pk_data is OmniSecp256k1Data(pub [u8;64])
                let secp_primitive_pk = near_crypto::Secp256K1PublicKey::try_from(omni_pk_data.0.as_slice())
                    .map_err(|e| eyre!("Failed to convert Omni Secp256k1 PK data to primitive Secp256k1PublicKey: {:?}", e))?;
                near_crypto::PublicKey::SECP256K1(secp_primitive_pk)
            }
        };

        let primitive_block_hash = near_primitives::hash::CryptoHash(omni_tx.block_hash.0);

        let primitive_actions: Vec<near_primitives::transaction::Action> = omni_tx.actions.into_iter().map(|omni_a| {
            match omni_a {
                OmniAction::FunctionCall(fc) => {
                    near_primitives::transaction::Action::FunctionCall(Box::new(near_primitives::transaction::FunctionCallAction {
                        method_name: fc.method_name,
                        args: fc.args,
                        gas: fc.gas.0,
                        deposit: fc.deposit.0,
                    }))
                }
                // Add other action conversions if needed, for this test FunctionCall is enough
                _ => unimplemented!("Action type conversion not implemented for this test"),
            }
        }).collect();

        let transaction_primitive = Transaction::V0(TransactionV0 {
            signer_id: primitive_signer_id,
            public_key: primitive_public_key,
            nonce: omni_tx.nonce.0, // omni_tx.nonce is U64(u64)
            receiver_id: primitive_receiver_id,
            block_hash: primitive_block_hash,
            actions: primitive_actions,
        });

        // 7. Sign and Send
        let tx_hash = transaction_primitive.get_hash_and_size().0;
        let signature = signer.sign(tx_hash.as_ref());
        let signed_transaction_primitive = near_primitives::transaction::SignedTransaction::new(signature, transaction_primitive.clone());

        info!("Attempting to send OMNI-built transaction: {:?}", signed_transaction_primitive);
        let outcome_result = runtime.send_near_transaction(signed_transaction_primitive).await;

        assert!(outcome_result.is_ok(), "send_near_transaction with omni-built failed: {:?}", outcome_result.err());
        let outcome = outcome_result.unwrap();
        info!("OMNI-built Transaction outcome: {:?}", outcome);
        match outcome.status {
            FinalExecutionStatus::SuccessValue(value) => {
                info!("OMNI-built Transaction successful, return value (if any): {:?}", String::from_utf8_lossy(&value));
            }
            _ => {
                panic!("OMNI-built Transaction was not successful: {:?}", outcome.status);
            }
        }
        Ok(())
    }

}

