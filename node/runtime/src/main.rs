
mod reencrypt;
mod llm;
mod tee_attestation;
mod evm;

use llm::{
    read_agent_secrets,
    connect_to_anthropic,
    connect_to_deepseek,
};
use reencrypt::run_reencrypt_example;
use rig::completion::Prompt;
use evm::{
    AppState,
    TransactionRequest,
    StorageQuery,
    deploy_contract,
    get_storage,
    get_1upnetwork_contract_bytecode,
};


#[tokio::main]
async fn main() -> color_eyre::Result<()> {

    color_eyre::install()?;

	let _ = tracing_subscriber::FmtSubscriber::builder()
		.with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
		.try_init();


    // run_reencrypt_example();

    let (
        tee_attestation_quote,
        tee_attestation_bytes
    ) = tee_attestation::generate_tee_attestation(true)?;


    let agent_secrets = read_agent_secrets(0);
    // Claude
    let (
        claude,
        agent
    ) = connect_to_anthropic(&agent_secrets.anthropic_api_key.unwrap());

    //// DeepSeek
    let agent = connect_to_deepseek(agent_secrets.deepseek_api_key.unwrap()).await;
    // let ask = &format!("Is the following data a valid TDX QuoteV4 trusted execution environment attestation?\n{:?}", tee_attestation_quote);

    // let ask = "does an LLM have a soul?";
    // println!("\nAsking: {}\nWaiting for LLM response...", ask);

    // let answer = agent.prompt(ask).await?;
    // println!("\nAnswer: {}", answer);

    let app_state = AppState::new();

    // Test running an EVM
    let deployment = deploy_contract(
        &app_state,
        TransactionRequest {
            // Contract bytecode (hex string)
            bytecode: get_1upnetwork_contract_bytecode(),
            // Transaction input (hex string)
            calldata: "".to_string(),
            // Sender address (hex string)
            sender: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
            // Ether value in wei
            value: 0,
        }
    );

    if let Ok((contract_addr, _contract_data)) = deployment {

        let json_data = get_storage(
            &app_state,
            StorageQuery {
                contract: contract_addr,
                slot: "0x0".to_string()
            }
        );
        println!("Data from contract: {:?}", json_data);
    }


    Ok(())
}

