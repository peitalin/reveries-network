use std::fmt::Display;
use regex::Regex;
use lazy_static::lazy_static;
use libp2p::gossipsub::{self, IdentTopic};
pub use libp2p::gossipsub::TopicHash;
use libp2p::PeerId;
use runtime::llm::AgentSecretsJson;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::create_network::NODE_SEED_NUM;
use crate::types::reverie::ReverieIdToAgentName;
pub type AgentName = String;
pub type AgentNonce = usize;
pub type FragmentNumber = usize;
pub type TotalFragments = usize;

lazy_static! {
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

    pub fn make_next_agent(&self) -> AgentNameWithNonce {
        let next_nonce = self.1 + 1;
        AgentNameWithNonce(
            self.0.clone(),
            next_nonce
        )
    }

    pub fn nonce(&self) -> usize {
        self.1
    }

    pub fn to_reverie_id(&self) -> ReverieIdToAgentName {
        ReverieIdToAgentName::from(self.clone())
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
