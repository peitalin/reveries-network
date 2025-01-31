use std::fmt::Display;
use regex::Regex;
use libp2p::gossipsub::{self, IdentTopic, TopicHash};
use serde::{Deserialize, Serialize};
use crate::event_loop::TopicSwitch;
use crate::{AgentName, FragmentNumber, AGENT_DELIMITER};


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipTopic {
    BroadcastKfrag(AgentName, FragmentNumber),
    TopicSwitch(TopicSwitch),
    // catches all other topics
    Chat(String),
}

impl Display for GossipTopic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = AGENT_DELIMITER;
        match self {
            Self::BroadcastKfrag(agent_name, frag_num) => {
                write!(f, "kfrag{}{}{}", frag_num, d, agent_name)
            },
            Self::TopicSwitch(TopicSwitch {
                prev_topic,
                next_topic,
                prev_vessel_peer_id
            }) => {
                write!(f, "topic_switch{}{}{}{}{}{}",
                    d, prev_topic,
                    d, next_topic,
                    d, prev_vessel_peer_id
                )
            },
            Self::Chat(s) => {
                write!(f, "{}", s)
            }
        }
    }
}

impl Into<String> for GossipTopic {
    fn into(self) -> String {
        format!("{}", self)
    }
}

impl From<String> for GossipTopic {
    fn from(s: String) -> Self {

        let (topic, agent_name, _) = split_topic_by_delimiter(&s);
        let agent_name = agent_name.to_string();

        if topic.starts_with("kfrag") {
            let frag_num = match topic.chars().last().expect("topic.chars.last err").to_digit(10) {
                None => panic!("kfrag topic should have a frag number: {}", topic),
                Some(n) => n
            };
            GossipTopic::BroadcastKfrag(agent_name, frag_num)
        } else {
            GossipTopic::Chat(s)
        }
    }
}

impl Into<IdentTopic> for GossipTopic {
    fn into(self) -> IdentTopic {
        gossipsub::IdentTopic::new(self.to_string())
    }
}

impl Into<TopicHash> for GossipTopic {
    fn into(self) -> TopicHash {
        <GossipTopic as Into<IdentTopic>>::into(self).into()
    }
}

impl From<TopicHash> for GossipTopic {
    fn from(i: TopicHash) -> GossipTopic {
        let s = i.to_string();
        s.into()
    }
}

// Temporary way to issue chat commands to the node and test features in development
// which will later be replaced with automated heartbeats, protocols, etc;
pub(crate) fn split_topic_by_delimiter(topic_str: &str) -> (&str, &str, Option<(usize, usize)>) {
    match topic_str {
        "chat" => ("chat", "", None),
        _ => {
            let mut tsplit = topic_str.split(AGENT_DELIMITER);
            let topic = tsplit.next().unwrap_or("unknown_topic");
            let agent_name = tsplit.next().unwrap_or("");
            let nshare_threshold: Option<(usize, usize)> = match tsplit.next() {
                None => None,
                Some(nt) => {
                    let re = Regex::new(r"\(([0-9]*),([0-9]*)\)").unwrap();
                    if let Some((_, [n, t])) = re.captures(nt).map(|c| c.extract()) {
                        let nshares = n.parse().unwrap();
                        let threshold = t.parse().unwrap();
                        Some((nshares, threshold))
                    } else {
                        None
                    }
                }
            };
            (topic, agent_name, nshare_threshold)
        }
    }
}


#[allow(non_snake_case)]
#[cfg(test)]
mod tests {
    use super::*;

    async fn test_display_gossiptopic() {

    }

    // #[tokio::test(start_paused = true)]
    // async fn duration_since_last_heartbeat__reads_correctly() {
    //     let heartbeat_data = HeartBeatData::new(10);
    //     tokio::time::advance(Duration::from_secs(10)).await;
    //     assert_eq!(
    //         heartbeat_data.duration_since_last_heartbeat(),
    //         Duration::from_secs(10)
    //     );
    // }

}