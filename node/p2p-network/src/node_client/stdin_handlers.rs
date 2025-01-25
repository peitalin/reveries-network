
use tokio::io::AsyncBufReadExt;
use futures::SinkExt;
use crate::behaviour::{ChatMessage, StdInputCommand};
use super::NodeClient;


impl NodeClient {

    pub async fn listen_and_handle_stdin(&mut self) {

        self.log(format!("Enter messages in the terminal and they will be sent to connected peers."));
        // Read full lines from stdin
        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();

        loop {
            tokio::select! {
                Ok(Some(line)) = stdin.next_line() => {

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
        }
    }
}