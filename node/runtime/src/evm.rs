use std::sync::{Arc, Mutex};
use serde::Deserialize;
use hex;
use color_eyre::{Result, eyre::anyhow};
use revm::{
    db::{InMemoryDB, CacheDB, EmptyDB, DatabaseRef},
    Evm,
    Database,
    interpreter::primitives::keccak256,
};
use revm::primitives::{
    ExecutionResult, TransactTo,
    Address, U256, B256,
};
use tracing_subscriber::filter::dynamic_filter_fn;


// Shared state with thread-safe EVM database
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<CacheDB<EmptyDB>>>,
}
impl AppState {
    pub fn new() -> Self {
        Self {
            db: Arc::new(Mutex::new(CacheDB::new(EmptyDB::default()))),
        }
    }
}

#[derive(Deserialize)]
pub struct TransactionRequest {
    pub bytecode: String,     // Contract bytecode (hex string)
    pub calldata: String,     // Transaction input (hex string)
    pub sender: String,       // Sender address (hex string)
    pub value: u128,          // Ether value in wei
}

#[derive(Deserialize)]
pub struct StorageQuery {
    pub contract: String,
    pub slot: String,
}


pub fn deploy_contract(state: &AppState, req: TransactionRequest) -> Result<(String, String)> {

    // // Initialize in-memory database
    // let db = InMemoryDB::default();
    let db = state.db.lock().unwrap();

    // Execute transaction
    // Configure EVM
    let mut evm = Evm::builder()
        .with_db(db.clone())
        .modify_tx_env(|tx| {
            // Parse inputs
            tx.caller = req.sender.parse().unwrap();
            tx.transact_to = TransactTo::Create;
            tx.data = hex::decode(&req.bytecode).unwrap().into();
            tx.value = tx.value.into();
            tx.gas_limit = 5_000_000; // Set appropriate gas limit

        })
        .build();

    let result = evm.transact_commit().unwrap();

    match result {
        ExecutionResult::Success { output, .. } => {
            println!("\nEVM result: {:?}", output);
            println!("\tCreated a contract at address: {}", output.address().unwrap());
            // hex::encode(output.into_data())
            Ok((output.address().unwrap().to_string(), hex::encode(output.data())))
        }
        ExecutionResult::Revert { output, .. } => {
            println!("Execution Revert: {:?}", output);
            Err(anyhow!(output))
        }
        ExecutionResult::Halt { reason, .. } => {
            println!("Execution Halt: {:?}", result);
            Err(anyhow!("{:?}", result))
        }
    }
}

pub(crate) fn get_1upnetwork_contract_bytecode() -> String {
    /***
    // SPDX-License-Identifier: MIT
    pragma solidity 0.8.26;

    contract StorageExample {
        string public name;

        constructor() {
            name = "1up-network";
        }
    }
    ***/
    let bytecode = "0x7f3175702d6e6574776f726b0000000000000000000000000000000000000000006000556001602f60003960016000f300";
    hex::encode(bytecode)
}


pub(crate) fn get_storage(state: &AppState, query: StorageQuery) -> serde_json::Value {

    let mut db = state.db.lock().unwrap();

    let contract = hex::decode(query.contract.trim_start_matches("0x")).unwrap();
    let contract = Address::from_slice(&contract);

    println!("Querying contract addr: {}", contract);

    let slot = U256::from_str_radix(query.slot.trim_start_matches("0x"), 16).unwrap();

    match db.storage(contract, slot.into()) {
        Ok(value) => {
            let value_str = format!("{}", value.to_string());
            serde_json::json!({ "value": value_str })
        }
        Err(_) => serde_json::json!({
            "value": "Storage slot not found"
        })
    }
}