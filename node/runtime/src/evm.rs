use std::sync::{Arc, Mutex};
use serde::Deserialize;
use hex;
use color_eyre::{Result, eyre::anyhow};
use revm::{
    db::{CacheDB, EmptyDBTyped, InMemoryDB},
    Database,
    DatabaseCommit,
    Evm,
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
    ResultAndState,
    TxEnv,
    Env,
};
use core::convert::Infallible;



// Shared state with thread-safe EVM database
#[derive(Clone)]
pub struct AppState {
    pub cache_db: CacheDB<EmptyDBTyped<Infallible>>,

    pub contract_addr: Option<Address>,
}
impl AppState {
    pub fn new(caller_address: &[u8]) -> Self {

        let caller = Address::from_slice(caller_address);
        // Initialize base database and caller's account
        let mut cache_db = InMemoryDB::default();
        cache_db.insert_account_info(
            caller,
            revm::primitives::AccountInfo {
                balance: U256::from(1e18),
                nonce: 0,
                code_hash: revm::primitives::KECCAK_EMPTY,
                code: None,
            },
        );

        Self {
            // db: Arc::new(Mutex::new(CacheDB::new(EmptyDB::default()))),
            cache_db,
            contract_addr: None,
        }
    }
}

#[derive(Deserialize)]
pub struct TransactionRequest {
    pub bytecode: &'static str,     // Contract bytecode (hex string)
    pub calldata: String,     // Transaction input (hex string)
    pub caller: Address,      // Sender address (hex string)
    pub value: u128,          // Ether value in wei
}

#[derive(Deserialize)]
pub struct QueryNumber {
    pub caller: Address,
    pub contract: Address,
}

#[derive(Deserialize)]
pub struct IncrementNumberQuery {
    pub caller: Address,
    pub contract: Address,
}

#[derive(Deserialize)]
pub struct SetNumberQuery {
    pub caller: Address,
    pub contract: Address,
    pub number: U256,
}

pub fn deploy_contract(mut app_state: AppState, req: TransactionRequest) -> Result<AppState> {

    let bytecode_hex = get_solidity_contract_bytecode();
    // Contract bytecode (remove the 0x prefix)
    let bytecode = Bytes::from(hex::decode(&bytecode_hex[2..]).unwrap());

    // Isolate EVM borrowing in a separate scope
    let (result, state, contract_address) = {
        let mut evm = Evm::builder()
            .with_db(&mut app_state.cache_db)
            .modify_tx_env(|tx| {
                tx.caller = req.caller;
                tx.transact_to = TransactTo::Create;
                tx.data = bytecode.clone();
                tx.value = U256::ZERO;
                tx.gas_price = U256::from(1);
                tx.gas_limit = 1e18 as u64;
            })
            .build();

        let ResultAndState {
            result,
            state
        }= evm.transact().unwrap();

        match result.clone() {
            ExecutionResult::Success { output, .. } => {
                println!("\nEVM result: {:?}", output);
                let addr = *output.address().unwrap();
                println!("\nCreated a contract at address: {:?}", addr);
                (result, state, addr)
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
    app_state.cache_db.commit(state);
    app_state.contract_addr = Some(contract_address);

    println!("Saving contract_addr: {:?}", app_state.contract_addr);
    println!("Committing state transition to AppState.cache_db...");

    // Verify contract exists in base database
    let contract_account = app_state.cache_db.basic(contract_address)
        .expect("Contract account missing after commit");

    let contract_code = app_state.cache_db
        .code_by_hash(contract_account.expect("contract==None").code_hash)
        .expect("Contract code missing");

    println!("Successfully deployed to: {:?}", contract_address);
    println!("Code size: {} bytes", contract_code.len());

    Ok(app_state)
}


// Deployed Bytecode of Counter.sol Contract
pub(crate) fn get_solidity_contract_bytecode() -> &'static str {
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



pub(crate) fn query_number(app_state: &mut AppState, query: QueryNumber) -> U256 {

    // Call number() function
    const NUMBER_SELECTOR: [u8; 4] = hex_literal::hex!("8381f58a");
    let call_data = Bytes::from(NUMBER_SELECTOR);

    let mut evm = Evm::builder()
        .with_db(&mut app_state.cache_db)
        .modify_env(|env| {
            env.tx.caller = query.caller;
            env.tx.transact_to = TransactTo::Call(query.contract);
            env.tx.data = call_data.clone();
            env.tx.value = U256::ZERO;
            env.tx.gas_limit = 9_000_000;
        })
        .build();

    let result = evm.transact().unwrap();
    let number = U256::from_be_slice(&result.result.output().unwrap());
    number
}


pub(crate) fn increment_number(
    app_state: &mut AppState,
    query: IncrementNumberQuery
) -> ExecutionResult {

    // Call increment()
    const INCREMENT_SELECTOR: [u8; 4] = hex_literal::hex!("d09de08a");
    let increment_data = Bytes::from(INCREMENT_SELECTOR);

    // create a scope so when evm.with_db(..) drops, it releases the borrow
    // on app_state.cache_db
    let ResultAndState { result, state } = {

        let mut evm = Evm::builder()
            .with_db(&mut app_state.cache_db)
            .modify_env(|env| {
                env.tx.caller = query.caller;
                env.tx.transact_to = TransactTo::Call(query.contract);
                env.tx.data = increment_data.clone();
                env.tx.value = U256::ZERO;
                env.tx.gas_limit = 1_000_000;
            })
            .build();

        let result = evm.transact().unwrap();

        match &result.result {
            ExecutionResult::Success { output, .. } => {
                println!("\nEVM result: {:?}", output);
                result
            }
            ExecutionResult::Revert { output, .. } => {
                panic!("Deployment Failed: {:?}", output);
            }
            ExecutionResult::Halt { reason, .. } => {
                panic!("Deployment Failed: {:?}", reason);
            }
        }
    };

    println!("Increment result: {:?}", result);
    app_state.cache_db.commit(state);
    result
}


pub(crate) fn set_number(
    app_state: &mut AppState,
    query: SetNumberQuery
) -> ExecutionResult {

    // Call setNumber(42)
    const SET_NUMBER_SELECTOR: [u8; 4] = hex_literal::hex!("3fb5c1cb");
    let data = Bytes::from(SET_NUMBER_SELECTOR);

    let new_number = U256::from(query.number);
    let mut call_data = data.to_vec();
    call_data.extend_from_slice(&new_number.to_be_bytes::<32>());

    let ResultAndState { result, state } = {

        let mut evm = Evm::builder()
            .with_db(&mut app_state.cache_db)
            .modify_env(|env| {
                env.tx.caller = query.caller;
                env.tx.transact_to = TransactTo::Call(query.contract);
                env.tx.data = Bytes::from(call_data);
                env.tx.value = U256::ZERO;
                env.tx.gas_limit = 1_000_000;
            })
            .build();

        let result = evm.transact().unwrap();

        match &result.result {
            ExecutionResult::Success { output, .. } => {
                println!("\nEVM result: {:?}", output);
                result
            }
            ExecutionResult::Revert { output, .. } => {
                panic!("Deployment Failed: {:?}", output);
            }
            ExecutionResult::Halt { reason, .. } => {
                panic!("Deployment Failed: {:?}", reason);
            }
        }
    };

    println!("setNumber result: {:?}", result);
    app_state.cache_db.commit(state);
    result
}


pub fn revm_test2() {

    let caller = Address::from_slice(&[0x1; 20]);
    // Initialize base database and caller's account
    // let mut base_db = InMemoryDB::default();
    // Create CacheDB wrapper for atomic state changes
    let mut cache_db = CacheDB::new(InMemoryDB::default());
    cache_db.insert_account_info(
        caller,
        revm::primitives::AccountInfo {
            balance: U256::from(1e18),
            nonce: 0,
            code_hash: revm::primitives::KECCAK_EMPTY,
            code: None,
        },
    );

    let bytecode_hex = get_solidity_contract_bytecode();
    // Contract bytecode (remove the 0x prefix)
    let bytecode = Bytes::from(hex::decode(&bytecode_hex[2..]).unwrap());

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