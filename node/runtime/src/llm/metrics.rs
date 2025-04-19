use serde::Deserialize;
use color_eyre::{Result, eyre::anyhow};
use serde_json::json;
use reqwest;
use tracing::{info, error};


/// Structure for token usage statistics
#[derive(Debug, Deserialize, Default, Clone)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}

/// Structure for an LLM API response
#[derive(Debug, Deserialize)]
pub struct LlmApiResponse {
    pub text: String,
    pub tokens: TokenUsage,
}

/// Result that tracks success and token usage
#[derive(Debug, Clone)]
pub struct LlmResult {
    pub text: String,
    pub tokens: TokenUsage,
}

/// Struct to hold results of MCP tool usage analysis
#[derive(Default)]
pub struct ToolUsageResult {
    pub used: bool,
    pub tool_name: String,
    pub location: String,
    pub results: String,
}

/// Struct to track success metrics for MCP tool usage
#[derive(Default)]
pub struct ToolUsageMetrics {
    attempts: usize,
    successes: usize,
    models_used: Vec<String>,
    locations_used: Vec<String>,
    tools_used: Vec<String>,
    total_tokens: TokenUsage,
}

impl ToolUsageMetrics {
    /// Record a successful tool usage
    pub fn record_success(&mut self, model: &str, tool_name: &str, location: &str) {
        self.successes += 1;
        self.models_used.push(model.to_string());
        if !location.is_empty() {
            self.locations_used.push(location.to_string());
        }
        self.tools_used.push(tool_name.to_string());
    }

    /// Record an attempt (successful or not)
    pub fn record_attempt(&mut self) {
        self.attempts += 1;
    }

    /// Add token usage
    pub fn add_tokens(&mut self, tokens: &TokenUsage) {
        self.total_tokens.input_tokens += tokens.input_tokens;
        self.total_tokens.output_tokens += tokens.output_tokens;
        self.total_tokens.total_tokens += tokens.total_tokens;
    }

    /// Generate a summary report
    pub fn generate_report(&self) -> String {
        let separator = "================================================================================";
        let mut report = format!("\n{}\n", separator);
        report.push_str(&format!("🛠️ MCP TOOL USAGE REPORT 🛠️\n"));
        report.push_str(&format!("Total success rate: {}/{} attempts\n", self.successes, self.attempts));

        // Token usage information
        report.push_str(&format!("Total tokens used: {}\n", self.total_tokens.total_tokens));
        report.push_str(&format!("- Input tokens: {}\n", self.total_tokens.input_tokens));
        report.push_str(&format!("- Output tokens: {}\n", self.total_tokens.output_tokens));

        if !self.models_used.is_empty() {
            report.push_str("Models that used tools:\n");
            for model in &self.models_used {
                report.push_str(&format!("- {}\n", model));
            }
        }

        if !self.locations_used.is_empty() {
            report.push_str("Locations queried:\n");
            for location in &self.locations_used {
                report.push_str(&format!("- {}\n", location));
            }
        }

        if !self.tools_used.is_empty() {
            report.push_str("Tools used:\n");
            for tool in &self.tools_used {
                report.push_str(&format!("- {}\n", tool));
            }
        }

        report.push_str(&format!("{}\n", separator));
        report
    }
}



pub async fn call_llm_api(api_type: &str, api_key: &str, prompt: &str, context: &str) -> Result<LlmResult> {
    let client = reqwest::Client::new();

    let api_url = format!("http://localhost:8000/{}", api_type);

    let payload = json!({
        "api_key": api_key,
        "prompt": prompt,
        "context": context
    });

    let response = client.post(&api_url)
        .json(&payload)
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(anyhow!("LLM API request failed: {}", error_text));
    }

    let response_data: LlmApiResponse = response.json().await?;

    Ok(LlmResult {
        text: response_data.text,
        tokens: response_data.tokens,
    })
}
