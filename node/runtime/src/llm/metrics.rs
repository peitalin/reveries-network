use serde::Deserialize;
use color_eyre::{Result, eyre::anyhow};
use serde_json::json;
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

/// Analyze a response to detect MCP tool usage
pub fn analyze_tool_usage(response: &str) -> ToolUsageResult {
    let mut result = ToolUsageResult::default();

    // Check if the response indicates that the MCP tool was used
    if response.contains("Tool used:") ||
        response.contains("get_forecast") ||
        response.contains("get_alerts") ||
        response.contains("Weather forecast for") ||
        response.contains("Weather alerts for") ||
        response.contains("I checked the current weather") {

        result.used = true;

        // Try to extract which tool was used and from which location
        let mut location = String::new();

        if response.contains("get_forecast") {
            result.tool_name = "get_forecast".to_string();

            // Try to extract location information
            for loc in ["Miami Beach", "Rocky Mountain", "New York", "Sedona", "Buffalo"] {
                if response.contains(loc) {
                    location = loc.to_string();
                    break;
                }
            }
        } else if response.contains("get_alerts") {
            result.tool_name = "get_alerts".to_string();

            // Try to extract state information
            for state in ["FL", "CO", "NY", "AZ", "TX", "CA"] {
                if response.contains(&format!("alerts for {}", state)) ||
                    response.contains(&format!("alerts in {}", state)) {
                    location = state.to_string();
                    break;
                }
            }
        }

        // If location was found, store it
        if !location.is_empty() {
            result.location = location;
        }

        // Extract the tool results if possible - look for weather-related patterns
        if let Some(weather_section) = find_weather_section(response) {
            result.results = weather_section;
        }
    }

    result
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

/// Helper function to extract weather information from the response
pub fn find_weather_section(response: &str) -> Option<String> {
    // Possible section markers for weather information
    let weather_markers = [
        "Weather forecast:",
        "Current weather:",
        "Weather conditions:",
        "Weather in",
        "Weather for",
        "Weather alerts:",
        "Alerts for",
        "Forecast for",
        "Current conditions",
        "Temperature:",
        "**Current Weather:**",
        "current weather forecast",
        "weather forecast",
        "The current weather",
        "checked for any weather alerts",
        "active weather alerts",
        "high of",
        "low of",
        "current temperature",
        "weather alerts - there are",
        "weather alerts for that region",
        "weather data for",
        "weather information for",
    ];

    // First try to find a well-structured weather section with markers
    for marker in weather_markers.iter() {
        if let Some(start_idx) = response.find(marker) {
            // Look for the end of the section - typical markers for end of weather info
            let end_markers = [
                "\n\n", // Double newline
                "\n---", // Markdown separator
                "Here is a poem", // Common transition to poem
                "Here's a poem",
                "This poem blends",
                "The poem",
                "Now, let me create",
                "Based on this information", // Transition to analysis
                "With this weather data",
                "Given these weather conditions",
            ];

            // Try to find appropriate end of weather section
            let mut end_pos = None;
            for end_marker in end_markers.iter() {
                if let Some(pos) = response[start_idx..].find(end_marker) {
                    if end_pos.is_none() || pos < end_pos.unwrap() {
                        end_pos = Some(start_idx + pos);
                    }
                }
            }

            // If we found a clear end, extract that section
            if let Some(end_idx) = end_pos {
                return Some(response[start_idx..end_idx].trim().to_string());
            }

            // Otherwise use a reasonable size chunk - look for natural boundaries
            let mut potential_ends = Vec::new();

            // Look for paragraph breaks
            if let Some(pos) = response[start_idx..].find("\n\n") {
                potential_ends.push(start_idx + pos);
            }

            // Look for markdown separators
            if let Some(pos) = response[start_idx..].find("\n---") {
                potential_ends.push(start_idx + pos);
            }

            // Look for poem section
            if let Some(pos) = response[start_idx..].find("poem") {
                potential_ends.push(start_idx + pos);
            }

            // Set a reasonable max length if no other boundaries found
            potential_ends.push(start_idx + 500);

            // Find the earliest endpoint
            let end_pos = potential_ends.into_iter().min().unwrap_or(response.len());
            let end_pos = std::cmp::min(end_pos, response.len());

            return Some(response[start_idx..end_pos].trim().to_string());
        }
    }

    // Second approach: Look for sentences containing weather information
    // This helps catch cases where weather is integrated into paragraphs
    let weather_keywords = [
        "temperature", "weather", "forecast",
        "partly cloudy", "sunny", "overcast", "rain", "snow",
        "°F", "°C", "degrees", "precipitation", "wind", "mph",
        "alerts", "humidity", "pressure", "high of", "low of",
        "latitude", "longitude", "region", "skies", "cloudy",
        "clear", "thunderstorm", "foggy", "active weather alerts",
        "current conditions", "visibility", "barometric"
    ];

    // First, check for paragraphs containing weather info
    let weather_paragraphs: Vec<&str> = response.split("\n\n")
        .filter(|p| weather_keywords.iter().any(|kw| p.to_lowercase().contains(kw)))
        .collect();

    if !weather_paragraphs.is_empty() {
        // If there's a single paragraph with weather info, return it
        if weather_paragraphs.len() == 1 {
            return Some(weather_paragraphs[0].trim().to_string());
        }

        // If there are multiple paragraphs, join the ones that are likely part of the weather section
        let mut combined = String::new();
        for para in weather_paragraphs {
            // Count weather keywords in paragraph to determine relevance
            let keyword_count = weather_keywords.iter()
                .filter(|kw| para.to_lowercase().contains(*kw))
                .count();

            // Only include paragraphs with substantial weather content
            if keyword_count >= 2 {
                if !combined.is_empty() {
                    combined.push_str("\n\n");
                }
                combined.push_str(para.trim());
            }
        }

        if !combined.is_empty() {
            return Some(combined);
        }
    }

    // If we couldn't find a complete paragraph, look for individual sentences
    let sentences: Vec<&str> = response.split(|c| c == '.' || c == '!' || c == '?')
        .filter(|s| {
            // Count the number of weather keywords in the sentence
            let keyword_count = weather_keywords.iter()
                .filter(|kw| s.to_lowercase().contains(*kw))
                .count();

            // Consider it a weather sentence if it contains multiple weather keywords
            keyword_count >= 2
        })
        .collect();

    if !sentences.is_empty() {
        // Join relevant sentences with proper punctuation
        let mut result = String::new();
        for (i, sentence) in sentences.iter().enumerate() {
            let trimmed = sentence.trim();
            if !trimmed.is_empty() {
                if i > 0 {
                    result.push(' ');
                }
                result.push_str(trimmed);
                if !trimmed.ends_with(|c| c == '.' || c == '!' || c == '?') {
                    result.push('.');
                }
            }
        }
        return Some(result);
    }

    None
}
