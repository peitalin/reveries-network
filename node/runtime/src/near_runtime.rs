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
use borsh::BorshSerialize;


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


}

