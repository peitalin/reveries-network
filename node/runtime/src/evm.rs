use std::sync::{Arc, Mutex};
use serde::Deserialize;
use hex;
use color_eyre::{Result, eyre::anyhow};
use revm::{
    db::{InMemoryDB, CacheDB, EmptyDB},
    Evm,
    Database,
    DatabaseCommit,
};
use revm::primitives::{
    Account,
    AccountInfo,
    ExecutionResult,
    TransactTo,
    Address,
    U256,
    Bytes,
    Bytecode,
    TxEnv,
    Env,
};
use std::str::FromStr;



// Shared state with thread-safe EVM database
#[derive(Clone)]
pub struct AppState {
    // pub db: Arc<Mutex<CacheDB<EmptyDB>>>,
    pub db: Arc<Mutex<InMemoryDB>>,
}
impl AppState {
    pub fn new() -> Self {
        Self {
            // db: Arc::new(Mutex::new(CacheDB::new(EmptyDB::default()))),
            db: Arc::new(Mutex::new(InMemoryDB::default())),
        }
    }
}

#[derive(Deserialize)]
pub struct TransactionRequest {
    pub bytecode: &'static str,     // Contract bytecode (hex string)
    pub calldata: String,     // Transaction input (hex string)
    pub sender: Address,      // Sender address (hex string)
    pub value: u128,          // Ether value in wei
}

#[derive(Deserialize)]
pub struct StorageQuery {
    pub contract: String,
    pub slot: String,
}

type Evm2 = Evm<'static, (), CacheDB<EmptyDB>>;

pub fn create_evm(app_state: &AppState) -> Evm2 {

    // // Initialize in-memory database
    // let db = InMemoryDB::default();
    let db = app_state.db.lock().unwrap();

    let evm = Evm::builder()
        .with_db(db.clone())
        .build();

    evm
}


pub fn deploy_contract(mut evm: Evm2, req: TransactionRequest) -> Result<(String, String)> {

    // set balance (for gas)
    let balance = U256::from(10u128.pow(18)); // 1 ETH for deployment
    let account_info = AccountInfo::from_balance(balance);
    evm.db_mut().insert_account_info(req.sender, account_info);

    println!("\nDeploying Counter.sol contract bytecode...");

    if let Some(account) = evm.db_mut().basic(req.sender).unwrap() {
        println!("Sender's balance: {}", account.balance);
    } else {
        println!("Sender account not found!");
    }

    let bytecode = Bytes::from(hex::decode(&req.bytecode[2..]).unwrap());
    // Setup execution environment
    evm.tx_mut().caller = req.sender;
    evm.tx_mut().transact_to = TransactTo::Create;
    evm.tx_mut().data = Bytes::from(bytecode);
    evm.tx_mut().value = U256::from(req.value);
    evm.tx_mut().gas_limit = 1e18 as u64; // Set appropriate gas limit
    evm.tx_mut().gas_price = U256::from(1);

    let result = evm.transact_commit().unwrap();

    match result {
        ExecutionResult::Success { output, .. } => {
            println!("\nEVM result: {:?}", output);
            println!("Created a contract at address: {}", output.address().unwrap());
            // hex::encode(output.into_data())
            Ok((output.address().unwrap().to_string(), hex::encode(output.data())))
        }
        ExecutionResult::Revert { .. } => {
            println!("Execution: {:?}", result);
            Err(anyhow!("{:?}", result))
        }
        ExecutionResult::Halt { .. } => {
            println!("Execution: {:?}", result);
            Err(anyhow!("{:?}", result))
        }
    }
}


// Deployed Bytecode of Counter.sol Contract
pub(crate) fn get_1upnetwork_contract_bytecode() -> &'static str {
    /***
    // SPDX-License-Identifier: UNLICENSED
    pragma solidity ^0.8.13;

    contract Counter {
        uint256 public number;

        function setNumber(uint256 newNumber) public {
            number = newNumber;
        }

        function increment() public {
            number++;
        }
    }
    ***/
    "0x6080604052348015600e575f80fd5b5060ec8061001b5f395ff3fe6080604052348015600e575f80fd5b5060043610603a575f3560e01c80633fb5c1cb14603e5780638381f58a14604f578063d09de08a146068575b5f80fd5b604d6049366004607d565b5f55565b005b60565f5481565b60405190815260200160405180910390f35b604d5f805490806076836093565b9190505550565b5f60208284031215608c575f80fd5b5035919050565b5f6001820160af57634e487b7160e01b5f52601160045260245ffd5b506001019056fea26469706673582212205058da2efddf916673e799a51f53ec7d6321ccd5f5924a75a4cbaacc4dd9a9ec64736f6c63430008190033"
}


pub(crate) fn get_storage(state: &AppState, query: StorageQuery) -> serde_json::Value {

    let mut db = state.db.lock().unwrap();

    let contract = hex::decode(query.contract.trim_start_matches("0x")).unwrap();
    let contract_address = Address::from_slice(&contract);

    println!("\nQuerying contract addr: {}", contract_address);

    // Verify the contract code is stored
    let contract_account = db.basic(contract_address).expect("AccountInfo");
    println!("contract_account: {:?}", contract_account);

    let contract_code = db.code_by_hash(contract_account.unwrap().code_hash).expect("code_by_hash err");
    println!("Contract code length: {} bytes", contract_code.len());

    let slot = U256::from_str_radix(query.slot.trim_start_matches("0x"), 16).unwrap();

    match db.storage(contract_address, slot.into()) {
        Ok(value) => {
            let value_str = format!("{}", value.to_string());
            serde_json::json!({ "value": value_str })
        }
        Err(_) => serde_json::json!({
            "value": "Storage slot not found"
        })
    }
}


pub fn revm_test2() {

    let bytecode_hex = get_1upnetwork_contract_bytecode();
    // Contract bytecode (remove the 0x prefix)
    let bytecode = Bytes::from(hex::decode(&bytecode_hex[2..]).unwrap());

    // Caller address (example)
    let caller = Address::from_slice(&[0x1; 20]);

    // Initialize base database and caller's account
    let mut base_db = InMemoryDB::default();
    base_db.insert_account_info(
        caller,
        revm::primitives::AccountInfo {
            balance: U256::from(1e18),
            nonce: 0,
            code_hash: revm::primitives::KECCAK_EMPTY,
            code: None,
        },
    );

    // Create CacheDB wrapper for atomic state changes
    let mut cache_db = CacheDB::new(&mut base_db);

    // Isolate EVM borrowing in a separate scope
    let (result, contract_address) = {
        let mut evm = Evm::builder()
            .with_db(&mut cache_db)
            .modify_tx_env(|tx| {
                tx.caller = caller;
                tx.transact_to = TransactTo::Create;
                tx.data = bytecode.clone();
                tx.value = U256::ZERO;
                tx.gas_price = U256::from(1);
                tx.gas_limit = 1e18 as u64;
            })
            .build();

        let result = evm.transact().unwrap();

        match result.result.clone() {
            ExecutionResult::Success { output, .. } => {
                println!("\nEVM result: {:?}", output);
                let addr = *output.address().unwrap();
                println!("Created a contract at address: {:?}", addr);
                (result, addr)
            }
            ExecutionResult::Revert { output, .. } => {
                panic!("Deployment Failed: {:?}", output);
            }
            ExecutionResult::Halt { reason, .. } => {
                panic!("Deployment Failed: {:?}", reason);
            }
        }
    };  // EVM instance drops here, releasing borrow

    // Now we can safely commit
    cache_db.commit(result.state);

    // Verify contract exists in base database
    let contract_account = cache_db.basic(contract_address)
        .expect("Contract account missing after commit");

    let contract_code = cache_db
        .code_by_hash(contract_account.expect("contract==None").code_hash)
        .expect("Contract code missing");

    println!("Successfully deployed to: {:?}", contract_address);
    println!("Code size: {} bytes", contract_code.len());


}