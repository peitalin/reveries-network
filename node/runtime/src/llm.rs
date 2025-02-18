use dotenv;

use libp2p::identity::{ed25519, secp256k1};
use color_eyre::Result;
use serde::{Deserialize, Serialize};

use rig::{
    agent::Agent, completion::Prompt, providers::{
        // openai,
        anthropic::{
            completion::CompletionModel, ClientBuilder, CLAUDE_3_SONNET
        },
        deepseek::{self, DeepSeekCompletionModel},
    }
};



pub fn read_agent_secrets(seed: usize) -> AgentSecretsJson {

    let agent_name = match seed {
        0 => "auron".to_string(),
        1 => "beatrix".to_string(),
        2 => "cid".to_string(),
        3 => "dagger".to_string(),
        4 => "ellone".to_string(),
        5 => "fang".to_string(),
        _ => "unnamed".to_string(),
    };

    dotenv::dotenv().ok();
    let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY").ok();
    let openai_api_key = std::env::var("OPENAI_API_KEY").ok();
    let deepseek_api_key = std::env::var("DEEPSEEK_API_KEY").ok();

    let keypair_secp256k1 = secp256k1::Keypair::generate();
    let keypair_ed25519 = ed25519::Keypair::generate();

    let social_accounts = serde_json::json!({
        "twitter": {
            "username": "asuka.x",
            "password": "123twitterpass",
        },
        "github": {
            "username": "asuka.git",
            "password": "123githubpass",
        }
    });

    AgentSecretsJson {
        agent_name: agent_name,
        agent_nonce: 0,
        corekey_secp256k1: AgentKeypair {
            public_key: hex::encode(keypair_secp256k1.public().to_bytes()),
            secret_key: hex::encode(keypair_secp256k1.secret().to_bytes()),
        },
        corekey_ed25519: AgentKeypair {
            public_key: hex::encode(keypair_ed25519.public().to_bytes()),
            secret_key: hex::encode(keypair_ed25519.secret().as_ref()),
        },
        anthropic_api_key: anthropic_api_key,
        openai_api_key: openai_api_key,
        deepseek_api_key: deepseek_api_key,
        social_accounts: social_accounts,
        context: "Your name is Larry, your profession is a pizza chef".to_string(),
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentKeypair {
    public_key: String,
    secret_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentSecretsJson {

    pub agent_name: String,

    pub agent_nonce: usize,

    /// seckp256k1 private key, used to encrypt sensitive agent logins/keys
    /// before posting on Github, IPFS, or broadcsting on the network's Kademlia DHT
    // core_secret_key: [u8; 32],
    corekey_secp256k1: AgentKeypair,

    /// Wallet secret key for chains using ED25519 based keys (Solana/NEAR)
    // core_secret_key_ed25519: Vec<u8>,
    corekey_ed25519: AgentKeypair,

    pub anthropic_api_key: Option<String>,

    pub openai_api_key: Option<String>,

    pub deepseek_api_key: Option<String>,

    pub social_accounts: serde_json::Value,

    pub context: String,
}

pub fn connect_to_anthropic(anthropic_api_key: &str, context: &str) -> (CompletionModel, Agent<CompletionModel>) {
    // Create client with specific version and beta features
    let client = ClientBuilder::new(anthropic_api_key)
        .anthropic_version("2023-06-01")
        .anthropic_beta("prompt-caching-2024-07-31")
        .build();

    // Create a completion model
    let claude = client.completion_model(CLAUDE_3_SONNET);

    // Or create an agent directly
    let agent = client
        .agent(CLAUDE_3_SONNET)
        .context(context)
        .build();

    return (claude, agent)
}

pub async fn test_claude_query(
    anthropic_api_key: String,
    question: &str,
    context: &str
) -> Result<String> {
    let (claude, agent) = connect_to_anthropic(&anthropic_api_key, context);
    let response = agent.prompt(question).await?;
    // println!("Anthropic Claude response: {:?}", response);
    Ok(response)
}

pub async fn connect_to_deepseek(
    deepseek_api_key: String,
    context: &str
) -> Agent<DeepSeekCompletionModel> {
    let deepseek_client = deepseek::Client::new(&deepseek_api_key);
    let agent = deepseek_client
        .agent("deepseek-chat")
        .context(context)
        .build();

    agent
}

pub async fn test_deepseek_query(
    deepseek_api_key: String,
    question: &str,
    context: &str
) -> Result<String> {

    let deepseek_client = deepseek::Client::new(&deepseek_api_key);
    let agent = deepseek_client
        .agent("deepseek-chat")
        .context(context)
        .build();

    let answer = agent.prompt(question).await?;
    Ok(answer)
}