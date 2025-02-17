use std::fmt::Display;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;
use tracing::warn;

use crate::types::{ChatMessage, TOPIC_DELIMITER};
use super::NodeClient;


impl<'a> NodeClient<'a> {
    pub async fn listen_and_handle_stdin(&mut self) {
        // Read full lines from stdin
        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        loop {
            tokio::select! {
                Ok(Some(line)) = stdin.next_line() => {
                    self.handle_stdin_commands(line).await
                },
            }
        }
    }
    /// Temporary feature for development purposes only.
    /// Will be replaced with an automated protocol
    async fn handle_stdin_commands(&mut self, line: String) {
        if line.trim().len() == 0 {
            warn!("Message needs to begin with: <topic>");
        } else {

            let line_split = line.split(" ").collect::<Vec<&str>>();
            let cmd = line_split[0];

            match cmd.to_string().into() {
                StdInputCommand::UnknownCmd(s) => {
                    warn!("Unknown command: '{}'", s);
                    println!("Input must begin with 'chat' or 'llm'");
                }
                StdInputCommand::ChatCmd => {
                    if line_split.len() < 2 {
                        warn!("Message needs 2 or more words: 'chat <message>'");
                    } else {

                        let message = line_split[1..].join(" ");

                        self.chat_cmd_sender.send(ChatMessage {
                            topic: cmd.to_string().into(),
                            message: message.to_string(),
                        }).await.ok();
                    }
                }
                StdInputCommand::LLM => {
                    if line_split.len() < 2 {
                        warn!("Message needs 2 or more words: 'llm <message>'");
                    } else {
                        let message = line_split[1..].join(" ");
                        self.ask_llm(&message).await;
                    }
                }
            }
        }
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StdInputCommand {
    ChatCmd,
    LLM,
    UnknownCmd(String),
}

impl Display for StdInputCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChatCmd => write!(f, "chat"),
            Self::LLM => write!(f, "llm"),
            Self::UnknownCmd(s) => write!(f, "{}", s),
        }
    }
}

// Temporary way to issue chat commands to the node and test features in development
// which will later be replaced with automated heartbeats, protocols, etc;
impl From<String> for StdInputCommand {
    fn from(s: String) -> Self {
        let mut tsplit = s.splitn(2, TOPIC_DELIMITER);
        let cmd = tsplit.next().unwrap_or("unknown");
        match cmd {
            "chat" => Self::ChatCmd,
            "llm" => Self::LLM,
            _ => Self::UnknownCmd(s)
        }
    }
}
