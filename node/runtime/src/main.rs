
mod reencrypt;
mod llm;
mod tee_attestation;
mod evm;
mod test_commands;

use test_commands::{Cmd, CliArgument};
use clap::Parser;
use reencrypt::run_reencrypt_example;
use evm::{
    deploy_contract, get_1upnetwork_contract_bytecode,
    query_number,
    increment_number,
    set_number,
    AppState,
    QueryNumber,
    IncrementNumberQuery,
    SetNumberQuery,
    TransactionRequest
};
use revm::primitives::{Address, U256};
use std::str::FromStr;


#[tokio::main]
async fn main() -> color_eyre::Result<()> {

    color_eyre::install()?;
	let _ = tracing_subscriber::FmtSubscriber::builder()
		.with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
		.try_init();

    let cmd = Cmd::parse();

    match cmd.argument {
        CliArgument::TestUmbral => {
            run_reencrypt_example()?;
        }
        CliArgument::TestTee => {
            let (
                tee_attestation_quote,
                tee_attestation_bytes
            ) = tee_attestation::generate_tee_attestation(true)?;
        }
        CliArgument::TestEvm => {
            // Test running an EVM
            let caller = Address::from_str("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap();
            // AppState contains DB
            let app_state = AppState::new(caller.as_slice());

            let mut app_state = deploy_contract(
                app_state, // DB
                TransactionRequest {
                    // Contract bytecode (hex string)
                    bytecode: get_1upnetwork_contract_bytecode(),
                    // Transaction input (hex string)
                    calldata: "".to_string(),
                    // Sender address (hex string)
                    caller: caller,
                    // Ether value in wei
                    value: 0,
                }
            ).unwrap();

            if let Some(contract) = app_state.contract_addr {

                // increment twice
                increment_number(&mut app_state, IncrementNumberQuery { caller, contract });
                increment_number(&mut app_state, IncrementNumberQuery { caller, contract });

                let current_number = query_number(
                    &mut app_state,
                    QueryNumber { caller, contract }
                );
                println!("\nCurrent number: {}", current_number);

                set_number(
                    &mut app_state,
                    SetNumberQuery {
                        caller,
                        contract,
                        number: U256::from(33)
                    }
                );

                let new_number = query_number(
                    &mut app_state,
                    QueryNumber { caller, contract }
                );

                println!("\nNew number: {}", new_number);
            }
        }
    }

    Ok(())
}

