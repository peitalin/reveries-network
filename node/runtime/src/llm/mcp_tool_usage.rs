use serde::{Deserialize, Serialize};

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct MCPToolUsageMetrics {
    total_input_tokens: u64,
    total_output_tokens: u64,
    total_cache_creation_tokens: u64,
    total_cache_read_tokens: u64,
    usage_records: Vec<UsageRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageRecord {
    pub request_id: String,
    pub timestamp: i64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub tool_name: Option<String>,
    pub tool_type: Option<String>,
    pub linked_tool_id: Option<String>,
    pub reverie_id: Option<String>,
    pub spender_address: Option<String>,
    pub spender_type: Option<String>,
}

impl MCPToolUsageMetrics {

    /// Add a usage record from database data
    pub fn add_usage_record(&mut self, record: UsageRecord) {
        self.total_input_tokens += record.input_tokens;
        self.total_output_tokens += record.output_tokens;
        self.total_cache_creation_tokens += record.cache_creation_tokens.unwrap_or(0);
        self.total_cache_read_tokens += record.cache_read_tokens.unwrap_or(0);

        self.usage_records.push(record);
    }

    /// Clear all usage data
    pub fn clear_usage_data(&mut self) {
        self.usage_records.clear();
        self.total_input_tokens = 0;
        self.total_output_tokens = 0;
        self.total_cache_creation_tokens = 0;
        self.total_cache_read_tokens = 0;
    }

    /// Generate a summary report
    pub fn generate_report(&self) -> String {
        let separator = "=================================";
        let mut report = format!("\n{}\n", separator);
        report.push_str(&format!("Usage Report from Database\n"));
        report.push_str(&format!("Total records: {}\n", self.usage_records.len()));
        report.push_str(&format!("Total input tokens: {}\n", self.total_input_tokens));
        report.push_str(&format!("Total output tokens: {}\n", self.total_output_tokens));
        report.push_str(&format!("Total cache creation tokens: {}\n", self.total_cache_creation_tokens));
        report.push_str(&format!("Total cache read tokens: {}\n", self.total_cache_read_tokens));

        if !self.usage_records.is_empty() {
            report.push_str("Recent usage records:\n");
            for (i, record) in self.usage_records.iter().take(5).enumerate() {
                report.push_str(&format!("{}. Request: {}, Tokens: {}/{}, Tool: {:?}\n",
                    i + 1,
                    record.request_id,
                    record.input_tokens,
                    record.output_tokens,
                    record.tool_name.as_deref().unwrap_or("None")));
            }
        }

        report.push_str(&format!("{}\n", separator));
        report
    }
}


