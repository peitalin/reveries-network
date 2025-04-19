mod metrics;

use color_eyre::Result;
use libp2p::identity::{ed25519, secp256k1};
use serde::{Deserialize, Serialize};
use tracing::info;
use reqwest;
pub use metrics::{
    ToolUsageMetrics,
    LlmResult,
    call_llm_api
};


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

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
    #[serde(rename = "type")]
    content_type: String,
}

pub async fn test_claude_query(
    anthropic_api_key: String,
    question: &str,
    context: &str
) -> Result<String> {
    let client = reqwest::Client::new();

    // Prepare the request payload
    let payload = serde_json::json!({
        "model": CLAUDE_3_SONNET,
        "system": context,
        "messages": [
            {
                "role": "user",
                "content": question
            }
        ],
        "max_tokens": 1000
    });

    // Make the API request
    let response = client.post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", anthropic_api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .await?;

    // Check if response is successful
    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(color_eyre::eyre::eyre!("Anthropic API error: {}", error_text));
    }

    // Parse the response
    let response_data: AnthropicResponse = response.json().await?;

    // Extract and return the response text
    let text = response_data.content.iter()
        .filter(|content| content.content_type == "text")
        .map(|content| content.text.clone())
        .collect::<Vec<String>>()
        .join("");

    Ok(text)
}

// Convenience functions for different LLM providers
pub async fn call_anthropic(
    api_key: &str,
    prompt: &str,
    context: &str,
    metrics: &mut ToolUsageMetrics
) -> Result<LlmResult> {

    let result = call_llm_api("anthropic", api_key, prompt, context).await?;

    // Add token usage to metrics
    info!("Claude token usage - Input: {}, Output: {}, Total: {}",
            result.tokens.input_tokens,
            result.tokens.output_tokens,
            result.tokens.total_tokens);

    metrics.add_tokens(&result.tokens);

    Ok(result)
}

pub async fn call_deepseek(
    api_key: &str,
    prompt: &str,
    context: &str,
    metrics: &mut ToolUsageMetrics
) -> Result<LlmResult> {

    let result = call_llm_api("deepseek", api_key, prompt, context).await?;

    // Add token usage to metrics
    info!("DeepSeek token usage - Input: {}, Output: {}, Total: {}",
            result.tokens.input_tokens,
            result.tokens.output_tokens,
            result.tokens.total_tokens);

    metrics.add_tokens(&result.tokens);

    Ok(result)
}