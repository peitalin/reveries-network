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
                StdInputCommand::RequestCfragsCmd(agent_name) => {

                    let cfrags_raw = self.request_cfrags(agent_name)
                        .await
                        .into_iter()
                        .map(|r| {
                            r.map_err(|e| anyhow!(e.to_string()))
                        })
                        .collect::<Vec<Result<Vec<u8>, Error>>>();

                    let (
                        verified_cfrags,
                        mut new_vessel_cfrags,
                        total_frags_received
                    ) = self.parse_cfrags(cfrags_raw);


                    println!("vvvvvvvvvv: {:?}", new_vessel_cfrags);
                    // get next vessel (can randomise as well)
                    let mut new_vessel_pk = new_vessel_cfrags.pop().unwrap();
                    let threshold = new_vessel_pk.threshold as usize;
                    self.log(format!("Received {}/{} required CapsuleFrags", total_frags_received, threshold));


                    // Bob opens the capsule by using at least `threshold` cfrags,
                    // and then decrypts the re-encrypted ciphertext.
                    match umbral_pre::decrypt_reencrypted(
                        &self.umbral_key.secret_key, // bob
                        &new_vessel_pk.alice_pk, // alice
                        &new_vessel_pk.capsule.as_mut().unwrap(),
                        verified_cfrags,
                        new_vessel_pk.ciphertext.as_mut().unwrap()
                    ) {
                        Ok(plaintext_bob) => {

                            let decrypted_data = serde_json::from_slice::<serde_json::Value>(&plaintext_bob)
                                .expect("error marshalling decrypted plaintext to JSON data");

                            let agent_secrets_str = serde_json::to_string_pretty(&decrypted_data)
                                .expect("to_string_pretty error");

                            self.log(format!("Decrypted (re-encrypted) agent data:\n{}", agent_secrets_str));

                            let agent_secrets_json = serde_json::from_slice::<AgentSecretsJson>(&plaintext_bob)
                                .expect("parse AgentSecretJson error");

                            // if let Some(anthropic_api_key) = agent_secrets_json.anthropic_api_key {
                            //     self.log(format!("Decrypted Anthropic key, querying Claude:"));
                            //     let _ = test_claude_query(anthropic_api_key).await;
                            // }

                        },
                        Err(e) => {
                            let node_name = get_node_name(&self.peer_id);
                            self.log(format!(">>> Err({})", e));
                            if (total_frags_received < threshold as u32) {
                                self.log(format!(">>> Not enough fragments. Need {threshold}, received {total_frags_received}"));
                            } else {
                                self.log(format!(">>> Not decryptable by user {} with: {}", node_name, self.umbral_key.public_key));
                                self.log(format!(">>> Only decryptable by new vessel with: {}", new_vessel_pk.bob_pk));
                            }
                        }
                    };

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
    RequestCfragsCmd(String),
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
            Self::RequestCfragsCmd(agent_name) => write!(f, "request{}{}", d, agent_name),
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
            Self::RequestCfragsCmd(a) => format!("request{}{}", d, a),
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
            "request" => Self::RequestCfragsCmd(agent_name),
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