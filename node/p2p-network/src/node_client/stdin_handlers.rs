use std::fmt::Display;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;

use crate::AGENT_DELIMITER;
use crate::types::{ChatMessage, split_topic_by_delimiter};
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
            let cmd = line_split[0];

            match cmd.to_string().into() {
                StdInputCommand::UnknownCmd(s) => {
                    self.log(format!("Unknown command: '{}'", s));
                    println!("Command must be 'chat', 'broadcast.<agent>', 'request.<agent>'...");
                }
                StdInputCommand::ChatCmd => {
                    if line_split.len() < 2 {
                        self.log(format!("Message needs 2 or more words: 'chat <message>'"));
                    } else {

                        let message = line_split[1..].join(" ");

                        self.chat_cmd_sender
                            .send(ChatMessage {
                                topic: cmd.to_string().into(),
                                message: message.to_string(),
                            }).await
                            .expect("ChatMessage receiver not to be dropped");
                    }
                }
                StdInputCommand::LLM(question) => {
                    if line_split.len() < 2 {
                        self.log(format!("Message needs 2 or more words: 'llm <message>'"));
                    } else {
                        let message = line_split[1..].join(" ");
                        self.ask_llm(&message).await;
                    }
                }
                StdInputCommand::BroadcastKfragsCmd(agent_name, n, t) => {
                    let _ = self.broadcast_kfrags(agent_name, n, t).await;
                }
                StdInputCommand::RespawnCmd(agent_name, prev_vessel_peer_id) => {
                    let _ = self.request_respawn(agent_name, prev_vessel_peer_id).await;
                }
            }
        }
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StdInputCommand {
    ChatCmd,
    LLM(String),
    UnknownCmd(String),
    BroadcastKfragsCmd(
        String, // Topic: agent_name
        usize,  // shares: number of Umbral MPC fragments
        usize   // threshold: number of required Umbral fragments
    ),
    RespawnCmd(String, Option<PeerId>),
}

impl Display for StdInputCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = AGENT_DELIMITER;
        match self {
            Self::ChatCmd => write!(f, "chat"),
            Self::LLM(s) => write!(f, "llm {}", s),
            Self::UnknownCmd(s) => write!(f, "{}", s),
            Self::BroadcastKfragsCmd(agent_name, n, t) => {
                write!(f, "broadcast{d}{}{d}({n},{t})", agent_name)
            }
            Self::RespawnCmd(agent_name, ..) => write!(f, "request{}{}", d, agent_name),
        }
    }
}

impl Into<String> for StdInputCommand {
    fn into(self) -> String {
        let d = AGENT_DELIMITER;
        match self {
            Self::ChatCmd => "chat".to_string(),
            Self::LLM(s) => s,
            Self::UnknownCmd(s) => s.to_string(),
            Self::BroadcastKfragsCmd(agent_name, n, t) => {
                format!("broadcast{d}{}{d}({n},{t})", agent_name)
            }
            Self::RespawnCmd(agent_name, prev_vessel_peer_id) => {
                format!("request{}{}", d, agent_name)
            }
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
            "llm" => Self::LLM(s),
            "request" => Self::RespawnCmd(agent_name, None),
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