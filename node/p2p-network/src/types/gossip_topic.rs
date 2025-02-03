use std::fmt::Display;
use regex::Regex;
use lazy_static::lazy_static; // 1.3.0
use libp2p::gossipsub::{self, IdentTopic, TopicHash};
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use nanoid::nanoid;
use crate::create_network::NODE_SEED_NUM;

pub type AgentName = String;
pub type AgentNonce = usize;
pub type FragmentNumber = u32;

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
        r"kfrag/frag_num=([0-9]*)/name=([a-zA-Z0-9-_]*)/nonce=([0-9]*)"
    ).unwrap();

    //// When broadcasting a topic_switch to a new agent, topic will be:
    // topic_switch/total_frags=<n>/name=<next_agent_name>/nonce=<agent_nonce>/prev_name=<prev_agent_name>/prev_nonce=<agent_nonce>/prev_peer_id=<prev_peer_id>
    static ref TOPIC_SWITCH_REGEX: Regex = Regex::new(
        r"topic_switch/total_frags=([0-9]*)/name=([a-zA-Z0-9-_]*)/nonce=([a-zA-Z0-9]*)([/a-zA-Z0-9-_/=]*)"
    ).unwrap();

    // For parsing the inner regex for topic_switch topics
    static ref PREV_TOPIC_REGEX: Regex = Regex::new(
        r"/prev_name=([a-zA-Z0-9-_]*)/prev_nonce=([a-zA-Z0-9]*)/prev_peer_id=([a-zA-Z0-9-_]*)"
    ).unwrap();
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
    agent_name: String,
    agent_nonce: usize,
    // total number of fragments in the new re-encryption of the key.
    // Nodes will do modular arithmetic private_seed % total_frags to get
    // their frag_num/channel to listen to for kfrags.
    pub total_frags: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrevTopic {
    agent_name: String,
    agent_nonce: usize,
    // To know which peer to remove from peer_info, peers_to_agent_frags, and kfrag_peers
    peer_id: Option<PeerId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipTopic {
    BroadcastKfrag(AgentName, AgentNonce, FragmentNumber),
    TopicSwitch(TopicSwitch),
    Unknown, // catches all other topics
}

impl Display for GossipTopic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = TOPIC_DELIMITER;

        //// When broadcasting kfrags, topic will be:
        // kfrag/frag_num=<frag_name>/name=<agent_name>/nonce=<agent_nonce>
        // kfrag/frag_num=3/name=bob-4uh1/nonce=1

        //// When broadcasting a topic_switch to a new agent, topic will be:
        // topic_switch/total_frags=<n>/name=<next_agent_name>/nonce=<agent_nonce>/prev_name=<prev_agent_name>/prev_nonce=<agent_nonce>/prev_peer_id=<prev_peer_id>

        // Then nodes will do modular arithmetic private_seed % 4 to get their frag_num (channel)
        // to listen to for kfrags:
        // let frag_num = NODE_SEED_NUM.with(|n| total_frags % *n.borrow());

        match self {
            Self::BroadcastKfrag(agent_name, agent_nonce, frag_num) => {
                write!(f, "kfrag/frag_num={frag_num}/name={agent_name}/nonce={agent_nonce}")
            },
            Self::TopicSwitch(TopicSwitch {
                next_topic,
                prev_topic,
            }) => {

                let total_frags = next_topic.total_frags;
                let next_topic2 = format!("name={}/nonce={}", next_topic.agent_name, next_topic.agent_nonce);

                match prev_topic {
                    Some(prev_topic) => {
                        let prev_topic2 = match prev_topic.peer_id {
                            None => format!("prev_name={}/prev_nonce={}", prev_topic.agent_name, prev_topic.agent_nonce),
                            Some(prev_peer_id) => {
                                format!(
                                    "prev_name={}/prev_nonce={}/prev_peer_id={}",
                                    prev_topic.agent_name,
                                    prev_topic.agent_nonce,
                                    prev_peer_id
                                )
                            }
                        };
                        // topic_switch/total_frags=<n>/name=<next_agent_name>/nonce=<agent_nonce>/prev_name=<prev_agent_name>/prev_nonce=<agent_nonce>/prev_peer_id=<prev_peer_id>
                        write!(f, "topic_switch/total_frags={total_frags}/{next_topic2}/{prev_topic2}")
                    }
                    None => {
                        write!(f, "topic_switch/total_frags={total_frags}/{next_topic2}")
                    }
                }
            }
            Self::Unknown => {
                write!(f, "Unknown")
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

        if let Some((_c, [
            frag_num,
            agent_name,
            agent_nonce
        ])) = KFRAG_REGEX.captures(&s).map(|c| c.extract()) {

            let frag_num = frag_num.parse::<u32>().ok().or(Some(0)).unwrap();
            let agent_nonce = agent_nonce.parse::<usize>().ok().or(Some(0)).unwrap();

            return GossipTopic::BroadcastKfrag(agent_name.to_string(), agent_nonce, frag_num)

        } else if let Some((_c, [
            total_frags,
            agent_name,
            agent_nonce,
            rest_of_str,
        ])) = TOPIC_SWITCH_REGEX.captures(&s).map(|c| c.extract()) {

            // next topic
            let total_frags  = total_frags.parse::<u32>().ok().or(Some(1)).unwrap();
            let agent_name = agent_name.to_string();
            let agent_nonce = agent_nonce.parse::<usize>().ok().or(Some(1)).unwrap();

            match PREV_TOPIC_REGEX.captures(&rest_of_str).map(|c| c.extract()) {
                None => {
                    return GossipTopic::TopicSwitch(TopicSwitch {
                        next_topic: NextTopic {
                            agent_name,
                            agent_nonce,
                            total_frags
                        },
                        prev_topic: None
                    })
                }
                Some((_c, [
                    prev_name,
                    prev_nonce,
                    prev_peer_id
                ])) => {
                    return GossipTopic::TopicSwitch(TopicSwitch {
                        next_topic: NextTopic {
                            agent_name,
                            agent_nonce,
                            total_frags
                        },
                        prev_topic: Some(PrevTopic {
                            agent_name: prev_name.to_string(),
                            agent_nonce: prev_nonce.parse::<usize>().ok().or(Some(0)).unwrap(),
                            peer_id: serde_json::from_str(&prev_peer_id).ok()
                        })
                    })
                }
            }
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
        let a = GossipTopic::TopicSwitch(
            TopicSwitch {
                next_topic: NextTopic {
                    agent_name: "bob-ef44".to_string(),
                    agent_nonce: 1,
                    total_frags: 4,
                },
                prev_topic: Some(PrevTopic {
                    agent_name: "alice-db22".to_string(),
                    agent_nonce: 2,
                    peer_id: Some(prev_peer_id),
                }),
            }
        );

        // Format:
        // topic_switch/total_frags=<n>/name=<next_agent_name>/nonce=<agent_nonce>/prev_name=<prev_agent_name>/prev_nonce=<agent_nonce>/prev_peer_id=<prev_peer_id>
        let expected_topic_str = format!("topic_switch/total_frags=4/name=bob-ef44/nonce=1/prev_name=alice-db22/prev_nonce=2/prev_peer_id={}", prev_peer_id);
        assert_eq!(a.to_string(), expected_topic_str);
    }

    #[test]
    fn test_display_gossiptopic2() {

        let a = GossipTopic::TopicSwitch(
            TopicSwitch {
                next_topic: NextTopic {
                    agent_name: "bob-ef44".to_string(),
                    agent_nonce: 1,
                    total_frags: 5
                },
                prev_topic: None,
            }
        );

        // Format:
        // topic_switch/total_frags=<n>/name=<next_agent_name>/nonce=<agent_nonce>/prev_name=<prev_agent_name>/prev_nonce=<agent_nonce>/prev_peer_id=<prev_peer_id>
        let expected_topic_str = format!("topic_switch/total_frags=5/name=bob-ef44/nonce=1");
        assert_eq!(a.to_string(), expected_topic_str);
    }

    #[test]
    fn test_display_gossiptopic3() {

        let teststr1 = format!("topic_switch/total_frags=4/name=bob-ef44/nonce=3/prev_name=alice-db22/prev_nonce=2/prev_peer_id=a123456");
        let teststr2 = format!("topic_switch/total_frags=4/name=bob-ef44/nonce=3");

        match TOPIC_SWITCH_REGEX.captures(&teststr1).map(|c| c.extract()) {
            None => panic!("no regex match on teststr1"),
            Some((_cap, [
                total_frags,
                name,
                nonce,
                the_rest
            ])) => {

                assert_eq!(total_frags, "4");
                assert_eq!(name, "bob-ef44");
                assert_eq!(nonce, "3");
                assert_eq!(the_rest, "/prev_name=alice-db22/prev_nonce=2/prev_peer_id=a123456");

                if the_rest.len() > 0 {
                    match PREV_TOPIC_REGEX.captures(&the_rest).map(|c| c.extract()) {
                        None => panic!("No match on PREV_TOPIC_REGEX when there should be"),
                        Some((
                            _cap,
                            [prev_name, prev_nonce, prev_peer_id]
                        )) => {
                            assert_eq!(prev_name, "alice-db22");
                            assert_eq!(prev_nonce, "2");
                            assert_eq!(prev_peer_id, "a123456");
                        }
                    }
                }
            }
        }

        match TOPIC_SWITCH_REGEX.captures(&teststr2).map(|c| c.extract()) {
            None => panic!("no regex match on teststr2"),
            Some((_cap, [total_frags, name, nonce, rest])) => {
                assert_eq!(total_frags, "4");
                assert_eq!(name, "bob-ef44");
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