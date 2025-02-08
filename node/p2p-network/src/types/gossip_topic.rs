use std::fmt::Display;
use regex::Regex;
use lazy_static::lazy_static;
use libp2p::gossipsub::{self, IdentTopic};
pub use libp2p::gossipsub::TopicHash;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use nanoid::nanoid;

use crate::create_network::NODE_SEED_NUM;

pub type AgentName = String;
pub type AgentNonce = usize;
pub type FragmentNumber = usize;
pub type TotalFragments = usize;

pub const TOPIC_DELIMITER: &'static str = "/";
const NANOID_ALPHABET: [char; 16] = [
    '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', 'a', 'b', 'c', 'd', 'e', 'f'
];
pub fn nanoid4() -> String {
    nanoid!(4, &NANOID_ALPHABET) //=> "4f90"
}

lazy_static! {
    //// When broadcasting kfrags, topic format is:
    // kfrag/frag_num=<frag_name>/name=<agent_name>/nonce=<agent_nonce>
    // kfrag/frag_num=3/name=bob-4uh1/nonce=1
    static ref KFRAG_REGEX: Regex = Regex::new(
        r"kfrag/frag_num=([0-9]*)/total_frags=([0-9]*)/name=([a-zA-Z0-9-._]*)/nonce=([0-9]*)"
    ).unwrap();

    //// When broadcasting a topic_switch to a new agent, topic will be:
    // topic_switch/total_frags=<n>/name=<next_agent_name>/nonce=<agent_nonce>/prev_name=<prev_agent_name>/prev_nonce=<agent_nonce>/prev_peer_id=<prev_peer_id>
    static ref TOPIC_SWITCH_REGEX: Regex = Regex::new(
        r"total_frags=([0-9]*)/threshold=([0-9]*)/name=([a-zA-Z0-9-._]*)/nonce=([a-zA-Z0-9]*)([/a-zA-Z0-9-_/=]*)"
    ).unwrap();

    // For parsing the inner regex for topic_switch topics
    static ref PREV_TOPIC_REGEX: Regex = Regex::new(
        r"/prev_name=([a-zA-Z0-9-._]*)/prev_nonce=([a-zA-Z0-9]*)/prev_peer_id=([a-zA-Z0-9-_]*)"
    ).unwrap();

    static ref AGENT_NAME_NONCE_REGEX: Regex = Regex::new(
        r"([a-zA-Z0-9-._]*)-([a-zA-Z0-9-._]*)"
    ).unwrap();
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentNameWithNonce(pub AgentName, pub AgentNonce);

impl AgentNameWithNonce {
    pub fn as_key(&self) -> String {
        self.to_string()
    }
}

impl Display for AgentNameWithNonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.0, self.1)
    }
}

impl Into<String> for AgentNameWithNonce {
    fn into(self) -> String {
        format!("{}-{}", self.0, self.1)
    }
}

impl From<String> for AgentNameWithNonce {
    fn from(s: String) -> Self {
        match AGENT_NAME_NONCE_REGEX.captures(&s).map(|c| c.extract()) {
            Some((_c, [name, nonce])) => {
                AgentNameWithNonce(name.to_string(), nonce.parse::<usize>().unwrap())
            }
            None => {
                AgentNameWithNonce("".to_string(), 0)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicSwitch {
    // subscribe to next_topic
    pub next_topic: NextTopic,
    // unsubscribe from prev_topic
    pub prev_topic: Option<PrevTopic>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextTopic {
    pub agent_name_nonce: AgentNameWithNonce,
    // total number of fragments in the new re-encryption of the key.
    // Nodes will do modular arithmetic private_seed % total_frags to get
    // their frag_num/channel to listen to for kfrags.
    pub total_frags: usize,
    pub threshold: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrevTopic {
    pub agent_name_nonce: AgentNameWithNonce,
    // To know which peer to remove from peer_info, peers_to_agent_frags, and kfrag_peers
    pub peer_id: Option<PeerId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipTopic {
    BroadcastKfrag(AgentNameWithNonce, TotalFragments, FragmentNumber),
    TopicSwitch,
    Unknown, // catches all other topics
}

impl Default for TopicSwitch {
    fn default() -> Self {
        Self {
            next_topic: NextTopic {
                agent_name_nonce: AgentNameWithNonce("default".to_string(), 0),
                total_frags: 3,
                threshold: 2
            },
            prev_topic: None,
        }
    }
}

impl TopicSwitch {
    fn get_next_subscribtion_topic(&self, frag_num: usize) -> String {
        GossipTopic::BroadcastKfrag(
            self.next_topic.agent_name_nonce.clone(),
            self.next_topic.total_frags,
            frag_num
        ).to_string()
    }

    fn get_prev_unsubscription_topic(&self, frag_num: usize) -> Option<String> {
        if let Some(prev_topic) = &self.prev_topic {
            Some(GossipTopic::BroadcastKfrag(
                prev_topic.agent_name_nonce.clone(),
                0, // Todo: not used, refactor
                frag_num
            ).to_string())
        } else {
            None
        }
    }
}

impl From<String> for TopicSwitch {
    fn from(s: String) -> Self {

        if let Some((_c, [
            total_frags,
            threshold,
            agent_name,
            agent_nonce,
            rest_of_str,
        ])) = TOPIC_SWITCH_REGEX.captures(&s).map(|c| c.extract()) {

            // next topic
            let total_frags  = total_frags.parse::<usize>().ok().or(Some(1)).unwrap();
            let threshold = threshold.parse::<usize>().ok().or(Some(1)).unwrap();
            let agent_name = agent_name.to_string();
            let agent_nonce = agent_nonce.parse::<usize>().ok().or(Some(1)).unwrap();
            let agent_name_nonce = AgentNameWithNonce(agent_name, agent_nonce);

            match PREV_TOPIC_REGEX.captures(&rest_of_str).map(|c| c.extract()) {
                None => {
                    return TopicSwitch {
                        next_topic: NextTopic {
                            agent_name_nonce,
                            total_frags,
                            threshold
                        },
                        prev_topic: None
                    }
                }
                Some((_c, [
                    prev_name,
                    prev_nonce,
                    prev_peer_id
                ])) => {
                    let prev_name = prev_name.to_string();
                    let prev_nonce = prev_nonce.parse::<usize>().ok().or(Some(1)).unwrap();
                    return TopicSwitch {
                        next_topic: NextTopic {
                            agent_name_nonce,
                            total_frags,
                            threshold
                        },
                        prev_topic: Some(PrevTopic {
                            agent_name_nonce: AgentNameWithNonce(prev_name, prev_nonce),
                            peer_id: match PeerId::from_str(&prev_peer_id) {
                                Ok(peer_id) => Some(peer_id),
                                Err(e) => {
                                    println!("Invalid PeerId, defaulting to None {:?}", e);
                                    None
                                }
                            }
                        })
                    }
                }
            }
        } else {
            return TopicSwitch::default()
        }
    }
}


impl Display for TopicSwitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {

        let next_topic = self.next_topic.clone();
        let total_frags = next_topic.total_frags;
        let threshold = next_topic.threshold;
        let next_topic2 = format!("name={}/nonce={}", next_topic.agent_name_nonce.0, next_topic.agent_name_nonce.1);

        //// When broadcasting a topic_switch to a new agent, topic will be: topic_switch,
        //// followed by TopicSwitch struct data:
        // total_frags=<n>/name=<next_agent_name>/nonce=<agent_nonce>/prev_name=<prev_agent_name>/prev_nonce=<agent_nonce>/prev_peer_id=<prev_peer_id>

        // Then nodes will do modular arithmetic to get their frag_num (channel)
        // to listen to for kfrags:
        // let frag_num = NODE_SEED_NUM.with(|n| *n.borrow() % total_frags);

        match &self.prev_topic {
            Some(prev_topic) => {
                let prev_topic2 = match prev_topic.peer_id {
                    None => format!("prev_name={}/prev_nonce={}", prev_topic.agent_name_nonce.0, prev_topic.agent_name_nonce.1),
                    Some(prev_peer_id) => {
                        format!(
                            "prev_name={}/prev_nonce={}/prev_peer_id={}",
                            prev_topic.agent_name_nonce.0,
                            prev_topic.agent_name_nonce.1,
                            prev_peer_id
                        )
                    }
                };
                // topic_switch total_frags=<n>/threshold=<t>/name=<next_agent_name>/nonce=<agent_nonce>/prev_name=<prev_agent_name>/prev_nonce=<agent_nonce>/prev_peer_id=<prev_peer_id>
                // topic_switch total_frags=3/threshold=2/name=bob/nonce=2/prev_name=bob/prev_nonce=1/prev_peer_id=123123
                write!(f, "total_frags={total_frags}/threshold={threshold}/{next_topic2}/{prev_topic2}")
            }
            None => {
                write!(f, "total_frags={total_frags}/threshold={threshold}/{next_topic2}")
            }
        }
    }
}

impl Display for GossipTopic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {

        //// When broadcasting kfrags, topic will be:
        // kfrag/frag_num=<frag_name>/name=<agent_name>/nonce=<agent_nonce>
        // kfrag/frag_num=3/name=bob-4uh1/nonce=1

        match self {
            Self::BroadcastKfrag(
                agent_name_nonce,
                total_frags,
                frag_num
            ) => {
                let name = agent_name_nonce.0.clone();
                let nonce = agent_name_nonce.1;
                write!(f, "kfrag/frag_num={frag_num}/total_frags={total_frags}/name={name}/nonce={nonce}")
            },
            Self::TopicSwitch => write!(f, "topic_switch"),
            Self::Unknown => write!(f, "unknown"),
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
        if let Some((_c, [
            frag_num,
            total_frags,
            agent_name,
            agent_nonce
        ])) = KFRAG_REGEX.captures(&s).map(|c| c.extract()) {

            let frag_num = frag_num.parse::<usize>().ok().or(Some(0)).unwrap();
            let agent_nonce = agent_nonce.parse::<usize>().ok().or(Some(0)).unwrap();
            let total_frags = total_frags.parse::<usize>().ok().or(Some(0)).unwrap();

            return GossipTopic::BroadcastKfrag(
                AgentNameWithNonce(agent_name.to_string(), agent_nonce),
                total_frags,
                frag_num
            )

        } else if s.starts_with("topic_switch") {
            return GossipTopic::TopicSwitch

         } else {
            return GossipTopic::Unknown
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

#[allow(non_snake_case)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_gossiptopic1() {

        let prev_peer_id = PeerId::random();
        let a = TopicSwitch {
            next_topic: NextTopic {
                agent_name_nonce: AgentNameWithNonce("bob".to_string(), 1),
                total_frags: 4,
                threshold: 3,
            },
            prev_topic: Some(PrevTopic {
                agent_name_nonce: AgentNameWithNonce("alice".to_string(), 2),
                peer_id: Some(prev_peer_id),
            }),
        };

        // Format:
        // total_frags=<n>/threshold=<t>/name=<next_agent_name>/nonce=<agent_nonce>/prev_name=<prev_agent_name>/prev_nonce=<agent_nonce>/prev_peer_id=<prev_peer_id>
        let expected_topic_str = format!("total_frags=4/threshold=3/name=bob/nonce=1/prev_name=alice/prev_nonce=2/prev_peer_id={}", prev_peer_id);
        assert_eq!(a.to_string(), expected_topic_str);
    }

    #[test]
    fn test_display_gossiptopic2() {

        let a = TopicSwitch {
            next_topic: NextTopic {
                agent_name_nonce: AgentNameWithNonce("bob".to_string(), 1),
                total_frags: 5,
                threshold: 3
            },
            prev_topic: None,
        };

        let expected_topic_str = format!("total_frags=5/threshold=3/name=bob/nonce=1");
        assert_eq!(a.to_string(), expected_topic_str);
    }

    #[test]
    fn test_display_gossiptopic3() {

        let teststr1 = format!("total_frags=4/threshold=2/name=bob/nonce=3/prev_name=alice/prev_nonce=2/prev_peer_id=a123456");
        let teststr2 = format!("total_frags=4/threshold=2/name=bob/nonce=3");

        match TOPIC_SWITCH_REGEX.captures(&teststr1).map(|c| c.extract()) {
            None => panic!("no regex match on teststr1"),
            Some((_cap, [
                total_frags,
                threshold,
                name,
                nonce,
                the_rest
            ])) => {

                assert_eq!(total_frags, "4");
                assert_eq!(threshold, "2");
                assert_eq!(name, "bob");
                assert_eq!(nonce, "3");
                assert_eq!(the_rest, "/prev_name=alice/prev_nonce=2/prev_peer_id=a123456");

                if the_rest.len() > 0 {
                    match PREV_TOPIC_REGEX.captures(&the_rest).map(|c| c.extract()) {
                        None => panic!("No match on PREV_TOPIC_REGEX when there should be"),
                        Some((
                            _cap,
                            [prev_name, prev_nonce, prev_peer_id]
                        )) => {
                            assert_eq!(prev_name, "alice");
                            assert_eq!(prev_nonce, "2");
                            assert_eq!(prev_peer_id, "a123456");
                        }
                    }
                }
            }
        }

        match TOPIC_SWITCH_REGEX.captures(&teststr2).map(|c| c.extract()) {
            None => panic!("no regex match on teststr2"),
            Some((_cap, [total_frags, threshold, name, nonce, rest])) => {
                assert_eq!(total_frags, "4");
                assert_eq!(threshold, "2");
                assert_eq!(name, "bob");
                assert_eq!(nonce, "3");
                assert_eq!(rest, "");
            }
        }
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