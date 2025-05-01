mod mcp_tool_usage;

use color_eyre::{Result, eyre::anyhow};
use libp2p::identity::{ed25519, secp256k1};
use serde::{Deserialize, Serialize};
use reqwest;
pub use mcp_tool_usage::MCPToolUsageMetrics;


pub const CLAUDE_3_SONNET: &str = "claude-3-sonnet-20240229";

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

pub fn read_agent_secrets(seed: usize) -> AgentSecretsJson {

    let agent_name = match seed {
        1 => "auron".to_string(),
        2 => "beatrix".to_string(),
        3 => "cid".to_string(),
        4 => "dagger".to_string(),
        5 => "edea".to_string(),
        6 => "fang".to_string(),
        7 => "gilgamesh".to_string(),
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
            "username": format!("{}-x", agent_name),
            "password": "123twitterpass",
        },
        "github": {
            "username": format!("{}.git", agent_name),
            "password": "123githubpass",
        }
    });

    AgentSecretsJson {
        agent_name: agent_name.clone(),
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
        context: format!("Your name is {}, your profession is a pizza chef", agent_name),
    }
}

/// Result that tracks success and token usage
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmResult {
    pub text: String,
}

pub async fn call_python_llm_server(
    api_type: &str,
    prompt: &str,
    context: &str,
    tools: Option<serde_json::Value>,
    stream: bool
) -> Result<LlmResult> {

    let client = reqwest::Client::new();
    let api_url = format!("http://localhost:8000/{}", api_type);
    let payload = serde_json::json!({
        "prompt": prompt,
        "context": context,
        "stream": stream,
        "tools": tools
    });

    let response = client.post(&api_url)
        .json(&payload)
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(anyhow!("LLM API request failed: {}", error_text));
    }

    let response_data: LlmResult = response.json().await?;
    Ok(response_data)
}

pub async fn call_anthropic(
    prompt: &str,
    context: &str,
    tools: Option<serde_json::Value>,
    stream: bool,
    metrics: &mut MCPToolUsageMetrics
) -> Result<LlmResult> {
    call_python_llm_server("anthropic", prompt, context, tools, stream).await
}

pub async fn call_deepseek(
    prompt: &str,
    context: &str,
    tools: Option<serde_json::Value>,
    stream: bool,
    metrics: &mut MCPToolUsageMetrics
) -> Result<LlmResult> {
    call_python_llm_server("deepseek", prompt, context, tools, stream).await
}