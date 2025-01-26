
mod reencrypt;
mod llm;
mod tee_attestation;

use libp2p::identity::{ed25519, secp256k1};
use llm::{AgentSecretsJson, read_agent_secrets};
use reencrypt::run_reencrypt_example;

use rig::providers::anthropic::{
    ClientBuilder,
    completion::CompletionModel,
    CLAUDE_3_SONNET
};


fn main() -> color_eyre::Result<()> {

    color_eyre::install()?;

	let _ = tracing_subscriber::FmtSubscriber::builder()
		.with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
		.try_init();

    let agent_secrets = read_agent_secrets(0);
    let anthropic_api_key = agent_secrets.anthropic_api_key;
    // let (claude, agent) = connect_to_anthropic(&anthropic_api_key);
    // let response = agent.prompt("does an LLM have a soul?").await?;
    // println!("anthropic response: {:?}", response);

    // run_reencrypt_example();

    tee_attestation::generate_tee_attestation()?;

    Ok(())
}

