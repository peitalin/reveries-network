use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use color_eyre::{Result, eyre::Error, eyre::anyhow};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;
use umbral_pre::VerifiedCapsuleFrag;

use runtime::llm::{AgentSecretsJson, test_claude_query};
use crate::{SendError, get_node_name, AGENT_DELIMITER};
use crate::behaviour::{split_topic_by_delimiter, CapsuleFragmentIndexed};
use crate::types::ChatMessage;
use super::NodeClient;


impl NodeClient {

    pub async fn listen_and_handle_stdin(&mut self) {

        self.log(format!("Enter messages in the terminal and they will be sent to connected peers."));
        // Read full lines from stdin
        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();

        loop {
            tokio::select! {
                Ok(Some(line)) = stdin.next_line() => {
                    self.handle_stdin_commands(line).await
                }
            }
        }
    }

    /// Temporary feature for development purposes only.
    /// Will be replaced with an automated protocol
    async fn handle_stdin_commands(&mut self, line: String) {
        if line.trim().len() == 0 {
            self.log(format!("Message needs to begin with: <topic>"));
        } else {

            let line_split = line.split(" ").collect::<Vec<&str>>();
            let topic = line_split[0];

            match topic.to_string().into() {
                StdInputCommand::UnknownCmd(s) => {
                    self.log(format!("Unknown topic: '{}'", s));
                    println!("Topic must be 'chat', 'broadcast.<agent>', 'request.<agent>'...");
                }
                StdInputCommand::ChatCmd => {
                    if line_split.len() < 2 {
                        self.log(format!("Message needs 2 or more words: '<topic> <message>'"));
                    } else {

                        let message = line_split[1..].join(" ");

                        self.chat_sender
                            .send(ChatMessage {
                                topic: topic.to_string().into(),
                                message: message.to_string(),
                            }).await
                            .expect("ChatMessage receiver not to be dropped");
                    }
                }
                StdInputCommand::BroadcastKfragsCmd(agent_name, n, t) => {
                    self.broadcast_kfrags(agent_name, n, t).await;
                }
                StdInputCommand::RequestRespawn(agent_name) => {
                    self.request_respawn(agent_name).await;
                }
            }
        }
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StdInputCommand {
    ChatCmd,
    UnknownCmd(String),
    BroadcastKfragsCmd(
        String, // Topic: agent_name
        usize,  // shares: number of Umbral MPC fragments
        usize   // threshold: number of required Umbral fragments
    ),
    RequestRespawn(String),
}

impl Display for StdInputCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = AGENT_DELIMITER;
        match self {
            Self::ChatCmd => write!(f, "chat"),
            Self::UnknownCmd(s) => write!(f, "{}", s),
            Self::BroadcastKfragsCmd(agent_name, n, t) => {
                write!(f, "broadcast{d}{}{d}({n},{t})", agent_name)
            }
            Self::RequestRespawn(agent_name) => write!(f, "request{}{}", d, agent_name),
        }
    }
}

impl Into<String> for StdInputCommand {
    fn into(self) -> String {
        let d = AGENT_DELIMITER;
        match self {
            Self::ChatCmd => "chat".to_string(),
            Self::UnknownCmd(a) => a.to_string(),
            Self::BroadcastKfragsCmd(a, n, t) => {
                format!("broadcast{d}{}{d}({n},{t})", a)
            }
            Self::RequestRespawn(a) => format!("request{}{}", d, a),
        }
    }
}

impl From<String> for StdInputCommand {
    fn from(s: String) -> Self {

        let (
            topic,
            agent_name,
            nshare_threshold
        ) = split_topic_by_delimiter(&s);

        let agent_name = agent_name.to_string();
        match topic {
            "chat" => Self::ChatCmd,
            "request" => Self::RequestRespawn(agent_name),
            "broadcast" => match nshare_threshold {
                Some((n, t)) => Self::BroadcastKfragsCmd(agent_name, n, t),
                None => {
                    println!("Wrong format. Should be: 'broadcast.<agent_name>.(nshares, threshold)'");
                    println!("Defaulting to (n=3, t=2) share threshold");
                    Self::BroadcastKfragsCmd(agent_name, 3, 2)
                }
            }
            _ => Self::UnknownCmd(s)
        }
    }
}