use serde::{Deserialize, Serialize};


#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct MCPToolUsageMetrics {
    attempts: usize,
    successes: usize,
    models_used: Vec<String>,
    locations_used: Vec<String>,
    tools_used: Vec<String>,
}

impl MCPToolUsageMetrics {
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

    /// Generate a summary report
    pub fn generate_report(&self) -> String {
        let separator = "===============================================================";
        let mut report = format!("\n{}\n", separator);
        report.push_str(&format!("🛠️ MCP TOOL USAGE REPORT 🛠️\n"));
        report.push_str(&format!("Total success rate: {}/{} attempts\n", self.successes, self.attempts));

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


