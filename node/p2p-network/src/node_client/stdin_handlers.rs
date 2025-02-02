use std::fmt::Display;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;

use crate::types::{ChatMessage, TOPIC_DELIMITER};
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
            println!("cmd::::: {}", cmd);
            println!("cmd::::: {:?}", cmd.to_string());
            println!("cmd::::: {:?}", StdInputCommand::from(cmd.to_string()));

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
                StdInputCommand::LLM => {
                    if line_split.len() < 2 {
                        self.log(format!("Message needs 2 or more words: 'llm <message>'"));
                    } else {
                        let message = line_split[1..].join(" ");
                        self.ask_llm(&message).await;
                    }
                }
                StdInputCommand::BroadcastKfragsCmd(agent_name, agent_nonce, n, t) => {
                    let _ = self.broadcast_kfrags(agent_name, agent_nonce, n, t).await;
                }
                StdInputCommand::RespawnCmd(agent_name, agent_nonce, prev_vessel_peer_id) => {
                    let _ = self.request_respawn(agent_name, agent_nonce, prev_vessel_peer_id).await;
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
    BroadcastKfragsCmd(
        String, // agent_name (Topic)
        usize,  // agent_nonce (Topic)
        usize,  // shares: number of Umbral MPC fragments
        usize   // threshold: number of required Umbral fragments
    ),
    RespawnCmd(
        String, // agent_name (Topic)
        usize,  // agent_nonce (Topic)
        Option<PeerId> // prev_vessel_peer_id
    ),
}

impl Display for StdInputCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = TOPIC_DELIMITER;
        match self {
            Self::ChatCmd => write!(f, "chat"),
            Self::LLM => write!(f, "llm"),
            Self::UnknownCmd(s) => write!(f, "{}", s),
            Self::BroadcastKfragsCmd(agent_name, agent_nonce, n, t) => {
                write!(f, "broadcast{d}{}{d}{}{d}({n},{t})", agent_name, agent_nonce)
            }
            Self::RespawnCmd(agent_name, ..) => write!(f, "request{}{}", d, agent_name),
        }
    }
}

impl Into<String> for StdInputCommand {
    fn into(self) -> String {
        let d = TOPIC_DELIMITER;
        match self {
            Self::ChatCmd => "chat".to_string(),
            Self::LLM => "llm".to_string(),
            Self::UnknownCmd(s) => s.to_string(),
            Self::BroadcastKfragsCmd(agent_name, agent_nonce, n, t) => {
                format!("broadcast{d}{}{d}{}{d}({n},{t})", agent_name, agent_nonce)
            }
            Self::RespawnCmd(agent_name, agent_nonce, prev_vessel_peer_id) => {
                format!("request{d}{}{d}{}", agent_name, agent_nonce)
            }
        }
    }
}

impl From<String> for StdInputCommand {
    fn from(s: String) -> Self {

        let (
            topic,
            agent_name,
            agent_nonce,
            nshare_threshold
        ) = parse_stdin_cmd(&s);

        let agent_name = agent_name.to_string();
        let agent_nonce = agent_nonce.unwrap_or(0);

        match topic {
            "chat" => Self::ChatCmd,
            "llm" => Self::LLM,
            "request" => Self::RespawnCmd(agent_name, agent_nonce, None),
            "broadcast" => {
                match nshare_threshold {
                    Some((n, t)) => Self::BroadcastKfragsCmd(agent_name, agent_nonce, n, t),
                    None => {
                        println!("Wrong format. Should be: 'broadcast.<agent_name>.(nshares, threshold)'");
                        println!("Defaulting to (n=3, t=2) share threshold");
                        Self::BroadcastKfragsCmd(agent_name, agent_nonce, 3, 2)
                    }
                }
            }
            _ => Self::UnknownCmd(s)
        }
    }
}


// Temporary way to issue chat commands to the node and test features in development
// which will later be replaced with automated heartbeats, protocols, etc;
pub(crate) fn parse_stdin_cmd(topic_str: &str) -> (&str, &str, Option<usize>, Option<(usize, usize)>) {

    let mut tsplit = topic_str.split(TOPIC_DELIMITER);
    let cmd = tsplit.next().unwrap_or("unknown");

    match cmd {
        "chat" => ("chat", "", None, None),
        "llm" => ("llm", "", None, None),
        "request" => {
            let mut tsplit = topic_str.split(TOPIC_DELIMITER);
            let cmd = tsplit.next().unwrap_or("unknown");
            let agent_name = tsplit.next().unwrap_or("");
            let agent_nonce = tsplit.next()
                .unwrap_or("0")
                .parse::<usize>().ok().or(Some(0));

            (cmd, agent_name, agent_nonce, None)
        }
        "broadcast" => {
            let mut tsplit = topic_str.split(TOPIC_DELIMITER);
            let cmd = tsplit.next().unwrap_or("unknown");
            let agent_name = tsplit.next().unwrap_or("");
            let agent_nonce = tsplit.next()
                .unwrap_or("0")
                .parse::<usize>().ok().or(Some(0));

            let nshare_threshold: Option<(usize, usize)> = match tsplit.next() {
                None => None,
                Some(nt) => {
                    let re = regex::Regex::new(r"\(([0-9]*),([0-9]*)\)").unwrap();
                    if let Some((_, [n, t])) = re.captures(nt).map(|c| c.extract()) {
                        let nshares = n.parse().unwrap();
                        let threshold = t.parse().unwrap();
                        Some((nshares, threshold))
                    } else {
                        None
                    }
                }
            };
            (cmd, agent_name, agent_nonce, nshare_threshold)
        }
        _ => ("unknown", "", None, None),
    }
}
