
use std::{
    collections::{HashMap, HashSet}, fmt::Display, str::FromStr
};
use futures::SinkExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;

use crate::AGENT_DELIMITER;
use crate::behaviour::{ChatMessage, split_topic_by_delimiter};
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

    async fn handle_stdin_commands(&mut self, line: String) {
        if line.trim().len() == 0 {
            self.log(format!("Message needs to begin with: <topic>"));
        } else {

            let line_split = line.split(" ").collect::<Vec<&str>>();
            let topic = line_split[0];

            match topic.to_string().into() {
                StdInputCommand::Unknown(s) => {
                    self.log(format!("Unknown topic: '{}'", s));
                    println!("Topic must be 'chat', 'broadcast.<agent>', 'request.<agent>'...");
                }
                StdInputCommand::Chat => {
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
                StdInputCommand::BroadcastKfrags(agent_name, n, t) => {
                    self.broadcast_kfrags(agent_name, n, t).await;
                }
                StdInputCommand::RequestCfrags(agent_name) => {
                    self.get_cfrags(agent_name).await;
                }
            }
        }
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StdInputCommand {
    Chat,
    Unknown(String),
    BroadcastKfrags(
        String, // Topic: agent_name
        usize,  // shares: number of Umbral MPC fragments
        usize   // threshold: number of required Umbral fragments
    ),
    RequestCfrags(String),
}

impl Display for StdInputCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = AGENT_DELIMITER;
        match self {
            Self::Chat => write!(f, "chat"),
            Self::Unknown(s) => write!(f, "{}", s),
            Self::BroadcastKfrags(agent_name, n, t) => {
                write!(f, "broadcast{d}{}{d}({n},{t})", agent_name)
            }
            Self::RequestCfrags(agent_name) => write!(f, "request{}{}", d, agent_name),
        }
    }
}

impl Into<String> for StdInputCommand {
    fn into(self) -> String {
        let d = AGENT_DELIMITER;
        match self {
            Self::Chat => "chat".to_string(),
            Self::Unknown(a) => a.to_string(),
            Self::BroadcastKfrags(a, n, t) => {
                format!("broadcast{d}{}{d}({n},{t})", a)
            }
            Self::RequestCfrags(a) => format!("request{}{}", d, a),
        }
    }
}

impl From<String> for StdInputCommand {
    fn from(s: String) -> Self {
        let (topic, agent_name, nshare_threshold) = split_topic_by_delimiter(&s);
        let agent_name = agent_name.to_string();
        match topic {
            "chat" => Self::Chat,
            "request" => Self::RequestCfrags(agent_name),
            "broadcast" => match nshare_threshold {
                Some((n, t)) => Self::BroadcastKfrags(agent_name, n, t),
                None => {
                    println!("Wrong format. Should be: 'broadcast.<agent_name>.(nshares, threshold)'");
                    println!("Defaulting to (n=3, t=2) share threshold");
                    Self::BroadcastKfrags(agent_name, 3, 2)
                }
            }
            _ => Self::Unknown(s)
        }
    }
}