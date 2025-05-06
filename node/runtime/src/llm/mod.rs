mod mcp_tool_usage;
mod agent_secrets_json;

use color_eyre::{Result, eyre::anyhow};
use serde::{Deserialize, Serialize};
use reqwest;
pub use mcp_tool_usage::MCPToolUsageMetrics;
pub use agent_secrets_json::{AgentSecretsJson, AgentKeypair, read_agent_secrets};




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