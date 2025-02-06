use std::fmt::Display;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;

use crate::types::{ChatMessage, PrevTopic, NextTopic, TopicSwitch, TOPIC_DELIMITER};
use super::NodeClient;


impl NodeClient {

    pub async fn listen_and_handle_stdin(&mut self) {
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
                    println!("Command must begin with 'topic_switch/', 'broadcast/', 'request/', etc");
                }
                StdInputCommand::ChatCmd => {
                    if line_split.len() < 2 {
                        self.log(format!("Message needs 2 or more words: 'chat <message>'"));
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
                        self.log(format!("Message needs 2 or more words: 'llm <message>'"));
                    } else {
                        let message = line_split[1..].join(" ");
                        self.ask_llm(&message).await;
                    }
                }
                StdInputCommand::Switch(topic_switch) => {
                    self.broadcast_switch_topic_nc(topic_switch).await.ok();
                }
                StdInputCommand::BroadcastKfragsCmd(agent_name, agent_nonce, n, t) => {
                    self.broadcast_kfrags(agent_name, agent_nonce, n, t).await.ok();
                }
                StdInputCommand::RespawnCmd(agent_name, agent_nonce, prev_vessel_peer_id) => {
                    self.request_respawn(agent_name, agent_nonce, prev_vessel_peer_id).await.ok();
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
    Switch(TopicSwitch),
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
        match self {
            Self::ChatCmd => write!(f, "chat"),
            Self::LLM => write!(f, "llm"),
            Self::UnknownCmd(s) => write!(f, "{}", s),
            Self::Switch(s) => {
                write!(f, "topic_switch")
            },
            Self::BroadcastKfragsCmd(agent_name, agent_nonce, n, t) => {
                write!(f, "broadcast/{}/{}/({n},{t})", agent_name, agent_nonce)
            }
            Self::RespawnCmd(agent_name, agent_nonce, prev_vessel_peer_id) => {
                write!(f, "request/{}/{}", agent_name, agent_nonce)
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
            nshare_threshold,
            prev_topic
        ) = parse_stdin_cmd(&s);

        let agent_name = agent_name.to_string();
        let agent_nonce = agent_nonce.unwrap_or(0);

        match topic {
            "chat" => Self::ChatCmd,
            "llm" => Self::LLM,
            "request" => Self::RespawnCmd(agent_name, agent_nonce, None),
            "topic_switch" => {

                // let topic_switch = TopicSwitch::from(s);
                let (total_frags, threshold)= nshare_threshold.unwrap_or((3,2));
                // n=3 shares, t=2 threshold default
                Self::Switch(TopicSwitch {
                    next_topic: NextTopic {
                        agent_name: agent_name,
                        agent_nonce: agent_nonce,
                        total_frags: total_frags,
                        threshold: threshold,
                    },
                    prev_topic: prev_topic,
                })
            }
            "broadcast" => {
                match nshare_threshold {
                    Some((n, t)) => Self::BroadcastKfragsCmd(agent_name, agent_nonce, n, t),
                    None => {
                        println!("Wrong format. Should be: 'broadcast/<agent_name>/<nonce>/(<nshares>,<threshold>)'");
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
pub(crate) fn parse_stdin_cmd(topic_str: &str) -> (
    &str,                   // command
    String,                 // agent name
    Option<usize>,          // agent nonce
    Option<(usize, usize)>, // (n,t)
    Option<PrevTopic>,      // prev topic
) {

    let mut tsplit = topic_str.splitn(2, TOPIC_DELIMITER);
    let cmd = tsplit.next().unwrap_or("unknown");
    let remainder_str = tsplit.next().unwrap_or("").to_string();

    match cmd {
        "chat" => ("chat", "".to_string(), None, None, None),
        "llm" => ("llm", "".to_string(), None, None, None),
        "topic_switch" => {

            let TopicSwitch {
                next_topic,
                prev_topic
            } = TopicSwitch::from(remainder_str);

            let agent_name2 = next_topic.agent_name.to_string();

            (cmd, agent_name2, Some(next_topic.agent_nonce), None, prev_topic)
        }
        "request" => {
            let mut tsplit = topic_str.split(TOPIC_DELIMITER);
            let cmd = tsplit.next().unwrap_or("unknown");
            let agent_name = tsplit.next().unwrap_or("").to_string();
            let agent_nonce = tsplit.next()
                .unwrap_or("0")
                .parse::<usize>().ok().or(Some(0));

            (cmd, agent_name, agent_nonce, None, None)
        }
        "broadcast" => {
            let mut tsplit = topic_str.split(TOPIC_DELIMITER);
            let cmd = tsplit.next().unwrap_or("unknown");
            let agent_name = tsplit.next().unwrap_or("").to_string();
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
            (cmd, agent_name, agent_nonce, nshare_threshold, None)
        }
        _ => ("unknown", "".to_string(), None, None, None),
    }
}
