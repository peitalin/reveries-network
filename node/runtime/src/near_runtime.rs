use color_eyre::{Result, eyre::eyre};
use tracing::{info, warn};
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use serde_json::json;

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
    },
    views::{
        QueryRequest,
        FinalExecutionOutcomeView,
        CallResult,
    },
};
use near_crypto::{InMemorySigner, SecretKey};

const DEFAULT_GAS: Gas = 30_000_000_000_000; // 30 TGas

#[derive(Clone, Debug)]
pub struct NearConfig {
    pub near_rpc_url: String,
}

const DEFAULT_NEAR_RPC_URL: &str = "https://rpc.testnet.near.org";

impl Default for NearConfig {
    fn default() -> Self {
        dotenv::dotenv().ok();
        Self {
            near_rpc_url: std::env::var("NEAR_RPC_URL").unwrap_or_else(|_| {
                tracing::debug!("NEAR_RPC_URL env var not set, defaulting to: {}", DEFAULT_NEAR_RPC_URL);
                DEFAULT_NEAR_RPC_URL.to_string()
            }),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReverieMetadata {
    pub reverie_type: String,
    pub description: String,
    pub access_condition: AccessCondition,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum AccessCondition {
    Umbral(String),
    Ecdsa(String),
    Ed25519(String),
    Contract {
        contract_account_id: String, // address of the contract
        user_account_id: String, // address of the user
        amount: u128, // amount of tokens
    },
}

#[derive(Clone)]
pub struct NearRuntime {
    near_client: JsonRpcClient,
}

impl NearRuntime {
    pub fn new(config: NearConfig) -> Result<Self> {
        info!("NEAR RPC URL: {}", config.near_rpc_url);
        let near_client = JsonRpcClient::connect(&config.near_rpc_url);

        Ok(Self {
            near_client,
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
            .map_err(|e| eyre!("RPC call failed for get_near_account_balance: {}", e))?;

        match response.kind {
            QueryResponseKind::ViewAccount(view) => {
                println!("{:#?}", view);
                Ok(view.amount)
            },
            _ => {
                Err(eyre!("Unexpected query response kind for account balance. Got: {:?}", response.kind))
            }
        }
    }

    pub async fn send_near_transaction(&self, signed_transaction: SignedTransaction) -> Result<FinalExecutionOutcomeView> {
        info!("Sending NEAR transaction...");
        let request = methods::broadcast_tx_commit::RpcBroadcastTxCommitRequest {
            signed_transaction,
        };
        let outcome = self.near_client.call(request).await
            .map_err(|e| eyre!("RPC call failed for send_near_transaction: {}", e))?;
        Ok(outcome)
    }

    pub async fn read_near_contract_state(
        &self,
        contract_id: &str,
        method_name: String,
        args: FunctionArgs
    ) -> Result<CallResult> {

        let contract_id = AccountId::from_str(contract_id)?;

        let query_request_payload = methods::query::RpcQueryRequest {
            block_reference: BlockReference::Finality(Finality::Final),
            request: QueryRequest::CallFunction {
                account_id: contract_id,
                method_name,
                args,
            },
        };

        let response = self.near_client.call(query_request_payload).await
            .map_err(|e| eyre!("RPC call failed for read_near_contract_state: {}", e))?;

        match response.kind {
            QueryResponseKind::CallResult(result) => Ok(result),
            _ => Err(eyre!("Unexpected query response kind for contract call. Got: {:?}", response.kind)),
        }
    }

    pub async fn _create_and_send_transaction(
        &self,
        signer_account_id_str: &str,
        signer_secret_key_str: &str,
        receiver_id_str: &str, // This will be the contract ID for contract calls
        actions: Vec<Action>,
    ) -> Result<FinalExecutionOutcomeView> {
        let signer_account_id = AccountId::from_str(signer_account_id_str)?;
        let signer_secret_key = SecretKey::from_str(signer_secret_key_str)?;
        let signer = InMemorySigner::from_secret_key(signer_account_id.clone(), signer_secret_key);
        let receiver_id = AccountId::from_str(receiver_id_str)?;

        // 1. Fetch latest nonce for the signer's access key
        let access_key_query_response = self.near_client.call(methods::query::RpcQueryRequest {
            block_reference: BlockReference::latest(),
            request: QueryRequest::ViewAccessKey {
                account_id: signer_account_id.clone(),
                public_key: signer.public_key(),
            },
        }).await.map_err(|e| eyre!("Failed to fetch access key nonce: {}", e))?;

        let current_nonce = match access_key_query_response.kind {
            QueryResponseKind::AccessKey(view) => view.nonce,
            _ => return Err(eyre!("Unexpected response kind for access key query: {:?}", access_key_query_response.kind)),
        };

        // 2. Fetch latest block hash
        let latest_block = self.near_client.call(methods::block::RpcBlockRequest {
            block_reference: BlockReference::latest(),
        }).await.map_err(|e| eyre!("Failed to fetch latest block: {}", e))?;
        let block_hash = latest_block.header.hash;

        // 3. Create Transaction
        let transaction_to_sign = Transaction::V0(near_primitives::transaction::TransactionV0 {
            signer_id: signer_account_id.clone(),
            public_key: signer.public_key(),
            nonce: current_nonce + 1,
            receiver_id: receiver_id.clone(),
            block_hash,
            actions,
        });

        // 4. Sign Transaction
        let (hash, _) = transaction_to_sign.get_hash_and_size();
        let signature = signer.sign(hash.as_ref());
        let signed_transaction = SignedTransaction::new(signature, transaction_to_sign);

        // 5. Send Transaction
        self.send_near_transaction(signed_transaction).await
    }
}

///////////////////////////
/// Payment Contract methods
///////////////////////////

impl NearRuntime {
    pub async fn deposit(
        &self,
        contract_id: &str,
        signer_account_id: &str,
        signer_secret_key: &str,
        reverie_id: &str,
        amount_to_deposit: Balance, // u128
    ) -> Result<FinalExecutionOutcomeView> {
        info!(
            "Calling deposit on contract {} with amount: {} for reverie {}",
            contract_id, amount_to_deposit, reverie_id
        );
        let args_json = json!({ "reverie_id": reverie_id });
        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "deposit".to_string(),
            args: args_json.to_string().into_bytes(),
            gas: DEFAULT_GAS,
            deposit: amount_to_deposit,
        }));
        self._create_and_send_transaction(
            signer_account_id,
            signer_secret_key,
            contract_id,
            vec![action],
        ).await
    }

    pub async fn get_balance(&self, contract_id: &str, reverie_id: &str, user_id: &str) -> Result<Balance> {
        info!("Calling get_balance on contract {} for reverie {} user: {}", contract_id, reverie_id, user_id);
        let _ = AccountId::from_str(user_id)?;
        let args_json = json!({ "reverie_id": reverie_id, "user_id": user_id });
        let args = FunctionArgs::from(args_json.to_string().into_bytes());

        let call_result = self.read_near_contract_state(
            contract_id,
            "get_balance".to_string(),
            args
        ).await?;

        let balance_str = serde_json::from_slice::<String>(&call_result.result)
            .map_err(|e| {
                let received_data = String::from_utf8_lossy(&call_result.result);
                eyre!(
                    "Failed to parse get_balance result (bytes: {:?}, utf8: '{}') as a JSON string: {}. Contract should return u128 as a string.",
                    call_result.result, received_data, e
                )
            })?;

        let balance = balance_str.parse::<u128>()?;
        Ok(balance)
    }

    pub async fn create_reverie(
        &self,
        contract_id: &str,
        signer_account_id: &str,
        signer_secret_key: &str,
        reverie_id: &str,
        reverie_type: &str,
        description: &str,
        access_condition: AccessCondition,
    ) -> Result<FinalExecutionOutcomeView> {

        let args_json = serde_json::json!({
            "reverie_id": reverie_id,
            "reverie_type": reverie_type,
            "description": description,
            "access_condition": access_condition
        });
        println!("Creating reverie with args: {}", args_json);

        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "create_reverie".to_string(),
            args: args_json.to_string().into_bytes(),
            gas: DEFAULT_GAS,
            deposit: 0,
        }));

        self._create_and_send_transaction(
            signer_account_id,
            signer_secret_key,
            contract_id,
            vec![action],
        ).await
    }

    pub async fn get_reverie_metadata(
        &self,
        contract_id: &str,
        reverie_id: &str,
    ) -> Result<Option<ReverieMetadata>> {
        let args_json = serde_json::json!({ "reverie_id": reverie_id });
        let args = FunctionArgs::from(args_json.to_string().into_bytes());
        let call_result = self.read_near_contract_state(
            contract_id,
            "get_reverie_metadata".to_string(),
            args
        ).await?;

        let metadata: Option<ReverieMetadata> = serde_json::from_slice(&call_result.result)
            .map_err(|e| eyre!("Failed to parse get_reverie_metadata result: {}", e))?;

        Ok(metadata)
    }

    pub async fn delete_all_reveries(
        &self,
        contract_id: &str,
        signer_account_id: &str,
        signer_secret_key: &str,
    ) -> Result<FinalExecutionOutcomeView> {
        let action_del = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "delete_all_reveries".to_string(),
            args: serde_json::json!({}).to_string().into_bytes(),
            gas: DEFAULT_GAS,
            deposit: 0,
        }));
        let outcome_del = self._create_and_send_transaction(
            signer_account_id,
            signer_secret_key,
            contract_id,
            vec![action_del],
        ).await?;
        info!("delete_all_reveries outcome: {:?}", outcome_del.status);
        Ok(outcome_del)
    }

    pub async fn get_reverie_ids(&self, contract_id: &str) -> Result<Vec<String>> {
        let args = FunctionArgs::from(json!({}).to_string().into_bytes());
        let call_result = self.read_near_contract_state(
            contract_id,
            "get_reverie_ids".to_string(),
            args
        ).await?;

        serde_json::from_slice(&call_result.result)
            .map_err(|e| eyre!("Failed to parse get_reverie_ids result: {}", e))
    }

    pub async fn can_spend(
        &self,
        contract_id: &str,
        reverie_id: &str,
        user_id: &str,
        amount_to_check: Balance, // u128
    ) -> Result<bool> {
        info!(
            "Calling can_spend on contract {} for reverie {} user: {} with amount: {}",
            contract_id, reverie_id, user_id, amount_to_check
        );
        let _ = AccountId::from_str(user_id)?; // Validate format
        let args_json = json!({
            "reverie_id": reverie_id,
            "user_id": user_id,
            "amount": amount_to_check.to_string()
        });
        let args = FunctionArgs::from(args_json.to_string().into_bytes());

        let call_result = self.read_near_contract_state(
            contract_id,
            "can_spend".to_string(),
            args
        ).await?;

        serde_json::from_slice(&call_result.result)
            .map_err(|e| eyre!("Failed to parse can_spend result: {}", e))
    }

    pub async fn record_spend(
        &self,
        contract_id: &str,
        signer_account_id: &str, // This should be the trusted account
        signer_secret_key: &str,
        reverie_id: &str,
        user_id: &str,
        amount_to_spend: Balance, // u128
    ) -> Result<FinalExecutionOutcomeView> {
        info!("Calling record_spend on contract {} for reverie {} user: {} amount: {}", contract_id, reverie_id, user_id, amount_to_spend);
        let _ = AccountId::from_str(user_id)?;
        let args_json = json!({
            "reverie_id": reverie_id,
            "user_id": user_id,
            "amount_to_spend": amount_to_spend.to_string()
        });
        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "record_spend".to_string(),
            args: args_json.to_string().into_bytes(),
            gas: DEFAULT_GAS,
            deposit: 0,
        }));
        self._create_and_send_transaction(
            signer_account_id,
            signer_secret_key,
            contract_id,
            vec![action],
        ).await
    }

    pub async fn get_trusted_account(&self, contract_id: &str) -> Result<String> {
        let args = FunctionArgs::from(json!({}).to_string().into_bytes());
        let call_result = self.read_near_contract_state(
            contract_id,
            "get_trusted_account".to_string(),
            args
        ).await?;

        let account_id_str: String = serde_json::from_slice(&call_result.result)
            .map_err(|e| eyre!("Failed to parse get_trusted_account result: {}", e))?;

        Ok(account_id_str)
    }

    pub async fn update_trusted_account(
        &self,
        contract_id: &str, // This signer must be the contract itself
        signer_secret_key: &str, // Secret key of the contract account
        new_trusted_account: &str,
    ) -> Result<FinalExecutionOutcomeView> {
        info!("Calling update_trusted_account on contract {} with new trusted: {}", contract_id, new_trusted_account);
        let _ = AccountId::from_str(new_trusted_account)?;
        let args_json = json!({ "new_trusted_account": new_trusted_account });
        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "update_trusted_account".to_string(),
            args: args_json.to_string().into_bytes(),
            gas: DEFAULT_GAS,
            deposit: 0,
        }));
        self._create_and_send_transaction(
            contract_id, // Signer is the contract account
            signer_secret_key,
            contract_id, // Receiver is also the contract account
            vec![action],
        ).await
    }

    pub async fn withdraw(
        &self,
        contract_id: &str,
        signer_account_id: &str,
        signer_secret_key: &str,
        reverie_id: &str,
        amount_to_withdraw: Balance, // u128
    ) -> Result<FinalExecutionOutcomeView> {
        info!(
            "Calling withdraw on contract {} for reverie {} account {} with amount: {}",
            contract_id, reverie_id, signer_account_id, amount_to_withdraw
        );

        let args_json = json!({
            "reverie_id": reverie_id,
            "amount": amount_to_withdraw.to_string()
        });

        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "withdraw".to_string(),
            args: args_json.to_string().into_bytes(),
            gas: DEFAULT_GAS,
            deposit: 0,
        }));

        self._create_and_send_transaction(
            signer_account_id,
            signer_secret_key,
            contract_id,
            vec![action],
        )
        .await
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use color_eyre::eyre::eyre;
    use dotenv::dotenv;
    use std::time::Duration;

    use near_primitives::transaction::{Transaction, TransactionV0};
    use near_primitives::views::FinalExecutionStatus;
    use near_token::NearToken;

    const TEST_CONTRACT_ID: &str = "payments.cyan-loong.testnet";
    const TEST_REVERIE_ID: &str = "test-reverie-1";

    fn setup_test_logger() {
        let _ = tracing_subscriber::fmt().with_env_filter("info").try_init();
    }

    #[tokio::test]
    async fn test_read_trusted_account() -> Result<()> {
        setup_test_logger();
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config)?;
        let trusted_account = runtime.get_trusted_account(TEST_CONTRACT_ID).await?;
        info!("Trusted account from contract {}: {}", TEST_CONTRACT_ID, trusted_account);
        assert!(!trusted_account.is_empty(), "Trusted account should not be empty");
        Ok(())
    }

    // #[tokio::test]
    // async fn test_send_omni_transaction_example() -> Result<()> {
    //     dotenv().ok();
    //     setup_test_logger();
    //     let config = NearConfig::default();
    //     let runtime = NearRuntime::new(config).await?;
    //     let (signer_id_str, signer_pk_str) = get_test_signer_info()?;

    //     use omni_transaction::TxBuilder;
    //     use omni_transaction::near::NearTransactionBuilder;
    //     use omni_transaction::near::types::{
    //         Action as OmniAction,
    //         FunctionCallAction as OmniFunctionCallAction,
    //         U64 as OmniU64,
    //         U128 as OmniU128,
    //         PublicKey as OmniPublicKey,
    //         ED25519PublicKey as OmniED25519Data,
    //         Secp256K1PublicKey as OmniSecp256k1Data,
    //         BlockHash as OmniBlockHash,
    //     };
    //     use near_jsonrpc_client::methods::query::RpcQueryRequest;

    //     let signer_account_id = AccountId::from_str(&signer_id_str)?;
    //     let signer_secret_key = SecretKey::from_str(&signer_pk_str)?;
    //     let signer = InMemorySigner::from_secret_key(signer_account_id.clone(), signer_secret_key.clone());

    //     runtime.delete_all_reveries(TEST_CONTRACT_ID, &signer_id_str, &signer_pk_str).await?;
    //     tokio::time::sleep(Duration::from_secs(2)).await;

    //     let reverie_id_omni = "omni-example-reverie";
    //     let access_condition_omni = AccessCondition::Umbral("omni_pubkey_example".to_string());

    //     let args_omni_json = serde_json::json!({
    //         "reverie_id": reverie_id_omni,
    //         "reverie_type": "omni_type",
    //         "description": "omni_description",
    //         "access_condition": access_condition_omni
    //     });

    //     let omni_action = OmniAction::FunctionCall(Box::new(
    //         OmniFunctionCallAction {
    //             method_name: "create_reverie".to_string(),
    //             args: args_omni_json.to_string().into_bytes(),
    //             gas: OmniU64(DEFAULT_GAS),
    //             deposit: OmniU128(0),
    //         }
    //     ));

    //     // 1. Fetch nonce
    //     let nonce_response = runtime.near_client.call(
    //         RpcQueryRequest {
    //             block_reference: BlockReference::latest(),
    //             request: QueryRequest::ViewAccessKey {
    //                 account_id: signer_account_id.clone(),
    //                 public_key: signer.public_key()
    //             }
    //         }
    //     ).await?;
    //     let current_nonce = match nonce_response.kind {
    //         QueryResponseKind::AccessKey(view) => view.nonce,
    //         _ => return Err(eyre!("Failed to get nonce, unexpected response kind: {:?}", nonce_response.kind))
    //     };
    //     let next_nonce = current_nonce + 1;

    //     // 2. Fetch block hash
    //     let block_hash_response = runtime.near_client.call(
    //         methods::block::RpcBlockRequest {
    //             block_reference: BlockReference::latest()
    //         }
    //     ).await?;
    //     let near_block_hash_primitive = block_hash_response.header.hash;

    //     // 3. Convert types for Omni Builder
    //     let omni_signer_public_key = match signer.public_key() {
    //         near_crypto::PublicKey::ED25519(data) => OmniPublicKey::ED25519(OmniED25519Data(data.0)),
    //         near_crypto::PublicKey::SECP256K1(data) => OmniPublicKey::SECP256K1(OmniSecp256k1Data(data.as_ref().try_into().unwrap())),
    //     };
    //     let omni_block_hash = OmniBlockHash(near_block_hash_primitive.0);

    //     let omni_tx_builder = NearTransactionBuilder::new();
    //     let omni_tx = omni_tx_builder
    //         .signer_id(signer_id_str.clone())
    //         .signer_public_key(omni_signer_public_key)
    //         .nonce(next_nonce)
    //         .receiver_id(TEST_CONTRACT_ID.to_string())
    //         .block_hash(omni_block_hash)
    //         .actions(vec![omni_action.clone()])
    //         .build();

    //     let primitive_public_key = match omni_tx.signer_public_key {
    //         OmniPublicKey::ED25519(pk_data) => {
    //             near_crypto::PublicKey::ED25519(near_crypto::ED25519PublicKey(pk_data.0))
    //         }
    //         OmniPublicKey::SECP256K1(pk_data) => {
    //             let secp_pk = near_crypto::Secp256K1PublicKey::try_from(pk_data.0.as_slice())?;
    //             near_crypto::PublicKey::SECP256K1(secp_pk)
    //         }
    //     };

    //     // 5. Build Omni Transaction
    //     let primitive_actions: Vec<Action> = omni_tx.actions.into_iter().map(|omni_a| {
    //         match omni_a {
    //             OmniAction::FunctionCall(fc) => {
    //                 Action::FunctionCall(Box::new(FunctionCallAction {
    //                     method_name: fc.method_name,
    //                     args: fc.args,
    //                     gas: fc.gas.0,
    //                     deposit: fc.deposit.0,
    //                 }))
    //             }
    //             _ => unimplemented!("Action type conversion not implemented for this test"),
    //         }
    //     }).collect();

    //     // 6. Convert Omni Transaction to Near Primitives Transaction
    //     let transaction_primitive = Transaction::V0(TransactionV0 {
    //         signer_id: omni_tx.signer_id,
    //         public_key: primitive_public_key,
    //         nonce: omni_tx.nonce.0,
    //         receiver_id: omni_tx.receiver_id,
    //         block_hash: near_primitives::hash::CryptoHash(omni_tx.block_hash.0),
    //         actions: primitive_actions,
    //     });

    //     // 7. Sign and Send
    //     let tx_hash = transaction_primitive.get_hash_and_size().0;
    //     let signature_omni = signer.sign(tx_hash.as_ref());
    //     let signed_transaction_primitive = SignedTransaction::new(signature_omni, transaction_primitive.clone());

    //     let outcome_result = runtime.send_near_transaction(signed_transaction_primitive).await;
    //     assert!(outcome_result.is_ok(), "send_near_transaction with omni-built failed: {:?}", outcome_result.err());
    //     let outcome = outcome_result.unwrap();
    //     info!("Omni-built Transaction outcome (create_reverie): {:?}", outcome.status);
    //     assert!(matches!(outcome.status, FinalExecutionStatus::SuccessValue(_)));

    //     runtime.delete_all_reveries(TEST_CONTRACT_ID, &signer_id_str, &signer_pk_str).await?;
    //     Ok(())
    // }

    // Helper to get signer details from env for tests
    fn get_test_signer_info() -> Result<(String, String)> {
        dotenv().ok();
        let signer_id = std::env::var("NEAR_SIGNER_ACCOUNT_ID")
            .map_err(|e| eyre!("NEAR_SIGNER_ACCOUNT_ID env var not found: {}", e))?;
        let signer_pk = std::env::var("NEAR_SIGNER_PRIVATE_KEY")
            .map_err(|e| eyre!("NEAR_SIGNER_PRIVATE_KEY env var not found: {}", e))?;
        Ok((signer_id, signer_pk))
    }

    ///////////////////////
    /// Reverie
    ///////////////////////

    #[tokio::test]
    #[serial_test::serial]
    async fn test_create_reverie() -> Result<()> {
        dotenv().ok();
        setup_test_logger();
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config)?;
        let (signer_id, signer_pk) = get_test_signer_info()?;

        let reverie_id_tx_example = "tx-example-reverie";
        runtime.delete_all_reveries(TEST_CONTRACT_ID, &signer_id, &signer_pk).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let access_condition_tx = AccessCondition::Ed25519("pubkey1".to_string());

        let outcome = runtime.create_reverie(
            TEST_CONTRACT_ID,
            &signer_id,
            &signer_pk,
            reverie_id_tx_example,
            "test_type_tx",
            "description_tx",
            access_condition_tx,
        ).await?;

        info!("Transaction outcome (create_reverie): {:?}", outcome.status);
        assert!(matches!(outcome.status, FinalExecutionStatus::SuccessValue(_)), "create_reverie in transaction example failed");

        runtime.delete_all_reveries(TEST_CONTRACT_ID, &signer_id, &signer_pk).await?;
        Ok(())
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_create_and_delete_reverie() -> Result<()> {
        setup_test_logger();
        let (signer_id, signer_pk) = get_test_signer_info()?;
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config)?;

        // Cleanup all reveries first
        runtime.delete_all_reveries(TEST_CONTRACT_ID, &signer_id, &signer_pk).await?;
        tokio::time::sleep(Duration::from_secs(2)).await; // Wait for delete to propagate

        // Create a reverie as trusted account
        let reverie_id = TEST_REVERIE_ID;
        let reverie_type = "Ed25519";
        let description = "desc";
        // let access_condition = AccessCondition::Ecdsa("pubkey".to_string());
        let access_condition = AccessCondition::Contract {
            contract_account_id: "payments.reveriesapp.testnet".to_string(),
            user_account_id: "some_user.testnet".to_string(),
            amount: 1_000,
        };

        // Dispatch create_reverie TX
        let outcome = runtime.create_reverie(
            TEST_CONTRACT_ID,
            &signer_id,
            &signer_pk,
            reverie_id,
            reverie_type,
            description,
            access_condition.clone(),
        ).await?;
        info!("create_reverie outcome: {:?}", outcome.status);
        assert!(matches!(outcome.status, FinalExecutionStatus::SuccessValue(_)), "create_reverie failed");
        tokio::time::sleep(Duration::from_secs(2)).await; // Wait for create to propagate

        // Try to create the same reverie again (should fail)
        let result_dup = runtime.create_reverie(
            TEST_CONTRACT_ID,
            &signer_id,
            &signer_pk,
            reverie_id,
            reverie_type,
            description,
            access_condition,
        ).await?;
        info!("create_reverie outcome: {:?}", result_dup.status);
        assert!(matches!(result_dup.status, FinalExecutionStatus::Failure(_)), "Duplicate create_reverie should fail");
        tokio::time::sleep(Duration::from_secs(2)).await; // Wait for this attempt's finality

        // Cleanup all reveries after test
        let outcome_del = runtime.delete_all_reveries(TEST_CONTRACT_ID, &signer_id, &signer_pk).await?;
        assert!(matches!(outcome_del.status, FinalExecutionStatus::SuccessValue(_)), "delete_all_reveries failed");

        tokio::time::sleep(Duration::from_secs(3)).await;
        let update_reverie_ids = runtime.get_reverie_ids(TEST_CONTRACT_ID).await?;
        info!("Reverie_ids should be empty: {:?}", update_reverie_ids);
        assert!(update_reverie_ids.is_empty(), "reverie_ids should be empty");

        Ok(())
    }

    ///////////////////////
    /// Can Spend / Record Spend
    ///////////////////////

    #[tokio::test]
    #[serial_test::serial]
    async fn test_contract_can_spend() -> Result<()> {
        setup_test_logger();
        let (signer_id, signer_pk) = get_test_signer_info()?; // Don't need PK for view call
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config)?;

        // Cleanup all reveries first
        runtime.delete_all_reveries(TEST_CONTRACT_ID, &signer_id, &signer_pk).await?;
        tokio::time::sleep(Duration::from_secs(3)).await;

        // Create a reverie as trusted account
        let reverie_type = "Ed25519";
        let description = "desc";
        let access_condition = AccessCondition::Ed25519("pubkey".to_string());

        // Dispatch create_reverie TX
        let outcome = runtime.create_reverie(
            TEST_CONTRACT_ID,
            &signer_id,
            &signer_pk,
            TEST_REVERIE_ID,
            reverie_type,
            description,
            access_condition.clone(),
        ).await?;
        info!("create_reverie outcome: {:?}", outcome.status);
        assert!(matches!(outcome.status, FinalExecutionStatus::SuccessValue(_)), "create_reverie failed");
        tokio::time::sleep(Duration::from_secs(3)).await;

        let user_to_check = signer_id.clone();
        let current_balance = runtime.get_balance(
            TEST_CONTRACT_ID,
            TEST_REVERIE_ID,
            &user_to_check
        ).await.unwrap_or(0);
        info!("Current balance for {} is {}", user_to_check, current_balance);

        if current_balance > 0 {
            let can_spend_some = runtime.can_spend(
                TEST_CONTRACT_ID,
                TEST_REVERIE_ID,
                &user_to_check,
                current_balance / 2
            ).await?;
            info!("can_spend(half): {}", can_spend_some);
            assert!(can_spend_some, "Should be able to spend half of current balance");

            let can_spend_exact = runtime.can_spend(
                TEST_CONTRACT_ID,
                TEST_REVERIE_ID,
                &user_to_check,
                current_balance
            ).await?;
            info!("can_spend(exact): {}", can_spend_exact);
            assert!(can_spend_exact, "Should be able to spend exact current balance");
        } else {
            info!("User balance is 0, skipping some can_spend checks that require balance.");
        }

        let can_spend_more = runtime.can_spend(
            TEST_CONTRACT_ID,
            TEST_REVERIE_ID,
            &user_to_check,
            current_balance + 1
        ).await?;
        info!("can_spend(more): {}", current_balance + 1);
        assert!(!can_spend_more, "Should NOT be able to spend more than current balance: {}", can_spend_more);

        let can_spend_zero = runtime.can_spend(
            TEST_CONTRACT_ID,
            TEST_REVERIE_ID,
            &user_to_check,
            0
        ).await?;
        info!("can_spend(zero): {}", can_spend_zero);
        assert!(can_spend_zero, "Should always be able to spend zero");

        Ok(())
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_contract_record_spend() -> Result<()> {
        setup_test_logger();
        let (signer_id, signer_pk) = get_test_signer_info()?;
        // This signer_id MUST be the contract's trusted_account for record_spend to succeed.
        // It will also be the user making the deposit and being debited in this test.
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config)?;

        let user_to_debit_str = signer_id.clone(); // Using signer_id as the user to debit
        let deposit_amount = NearToken::from_millinear(2).as_yoctonear(); // 0.002 NEAR
        let amount_to_spend = NearToken::from_millinear(1).as_yoctonear(); // 0.001 NEAR
        let amount_to_spend2: Balance = 1_000_000_000_000_000_000_000; // 1e21
        assert_eq!(amount_to_spend, amount_to_spend2);

        let initial_balance = runtime.get_balance(
            TEST_CONTRACT_ID,
            TEST_REVERIE_ID,
            &user_to_debit_str
        ).await?;
        info!("Initial balance for {} is {}", user_to_debit_str, initial_balance);

        // 1. Deposit funds for the user
        info!("Attempting to deposit {} yoctoNEAR for user {}", deposit_amount, user_to_debit_str);
        let deposit_outcome = runtime.deposit(
            TEST_CONTRACT_ID,
            &user_to_debit_str, // User deposits to their own account within the contract
            &signer_pk,         // User signs their own deposit
            TEST_REVERIE_ID,
            deposit_amount,
        ).await?;
        info!("Deposit outcome: {:?}", deposit_outcome.status);
        assert!(matches!(deposit_outcome.status, FinalExecutionStatus::SuccessValue(_)), "Deposit failed");

        // small delay or check balance to ensure deposit is processed if issues arise
        tokio::time::sleep(Duration::from_secs(1)).await;
        let balance_after_deposit = runtime.get_balance(
            TEST_CONTRACT_ID,
            TEST_REVERIE_ID,
            &user_to_debit_str
        ).await.unwrap_or(0);
        info!("Balance for {} after deposit: {}", user_to_debit_str, balance_after_deposit);
        assert!(balance_after_deposit >= deposit_amount, "Balance after deposit is less than deposit amount");

        // 2. Record the spend
        // The signer_id from get_test_signer_info() is used here and MUST be the trusted account.
        info!("Attempting record_spend for user {} amount {}", user_to_debit_str, amount_to_spend);
        let spend_outcome = runtime.record_spend(
            TEST_CONTRACT_ID,
            &signer_id,         // This MUST be the trusted account
            &signer_pk,         // PK for the trusted account
            TEST_REVERIE_ID,
            &user_to_debit_str, // User whose balance is being debited
            amount_to_spend
        ).await?;
        info!("record_spend outcome: {:?}", spend_outcome.status);
        assert!(matches!(spend_outcome.status, FinalExecutionStatus::SuccessValue(_)), "record_spend failed");

        // 3. Verify final balance
        info!("Waiting to ensure the spend is processed");
        tokio::time::sleep(Duration::from_secs(5)).await;
        let final_balance = runtime.get_balance(
            TEST_CONTRACT_ID,
            TEST_REVERIE_ID,
            &user_to_debit_str
        ).await?;
        info!("Final balance for {}: {}", user_to_debit_str, final_balance);
        assert_eq!(final_balance, balance_after_deposit - amount_to_spend, "user balance not updated after spend");

        info!("Attempting to withdraw all remaining balance");
        let withdraw_outcome = runtime.withdraw(
            TEST_CONTRACT_ID,
            &user_to_debit_str,
            &signer_pk,
            TEST_REVERIE_ID,
            final_balance,
        ).await?;
        info!("withdraw outcome: {:?}", withdraw_outcome.status);
        assert!(matches!(withdraw_outcome.status, FinalExecutionStatus::SuccessValue(_)), "withdraw failed");

        Ok(())
    }

    ///////////////////////
    /// Trusted Account
    ///////////////////////

    #[tokio::test]
    async fn test_contract_get_trusted_account() -> Result<()> {
        setup_test_logger();
        let config = NearConfig::default();
        let runtime = NearRuntime::new(config)?;
        let trusted_account = runtime.get_trusted_account(TEST_CONTRACT_ID).await?;
        info!("Contract trusted account: {}", trusted_account);
        // assert_eq!(trusted_account, TEST_CONTRACT_ID, "Trusted account mismatch");
        assert!(!trusted_account.is_empty(), "Trusted account should not be empty");
        Ok(())
    }

    #[tokio::test]
    #[ignore] // Ignored because it requires the test signer to be the contract account itself
    async fn test_contract_update_trusted_account() -> Result<()> {
        setup_test_logger();
        // For this test, signer_id must be TEST_CONTRACT_ID ("cyan-loong.testnet")
        // and signer_pk must be the private key for "cyan-loong.testnet"
        let (signer_id, signer_pk) = get_test_signer_info()?;
        if signer_id != TEST_CONTRACT_ID {
            warn!("Skipping test_contract_update_trusted_account: signer_id ({}) is not the contract_id ({}). This test requires signing as the contract itself.", signer_id, TEST_CONTRACT_ID);
            return Ok(());
        }

        let config = NearConfig::default();
        let runtime = NearRuntime::new(config)?;

        let original_trusted_account = runtime.get_trusted_account(TEST_CONTRACT_ID).await?;
        info!("Original trusted account: {}", original_trusted_account);

        let new_trusted_account = "new_trusted_friend.testnet".to_string();
        info!("Attempting to update trusted account to: {}", new_trusted_account);

        let outcome = runtime.update_trusted_account(
            TEST_CONTRACT_ID, // Signer is the contract itself
            &signer_pk,       // PK for the contract account
            &new_trusted_account,
        ).await?;
        info!("update_trusted_account outcome: {:?}", outcome.status);
        assert!(matches!(outcome.status, FinalExecutionStatus::SuccessValue(_)), "update_trusted_account failed");

        tokio::time::sleep(Duration::from_secs(2)).await;
        let fetched_new_trusted = runtime.get_trusted_account(TEST_CONTRACT_ID).await?;
        assert_eq!(fetched_new_trusted, new_trusted_account, "Trusted account was not updated");
        info!("Successfully updated trusted account to: {}", fetched_new_trusted);

        // Optional: Revert to original trusted account
        // let revert_outcome = runtime.update_trusted_account(TEST_CONTRACT_ID, &signer_pk, &original_trusted_account).await?;
        // assert!(matches!(revert_outcome.status, FinalExecutionStatus::SuccessValue(_)), "Failed to revert trusted account");
        // info!("Reverted trusted account to: {}", original_trusted_account);

        Ok(())
    }


}

