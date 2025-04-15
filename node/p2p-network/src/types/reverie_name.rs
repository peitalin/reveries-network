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
use crate::types::ReverieIdToNameKey;

pub type FragmentNumber = usize;
pub type TotalFragments = usize;

lazy_static! {
    static ref REVERIE_NAME_NONCE_REGEX: Regex = Regex::new(
        r"([a-zA-Z0-9-._]*)-([a-zA-Z0-9-._]*)"
    ).unwrap();
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReverieNameWithNonce(pub String, pub usize);

impl ReverieNameWithNonce {
    pub fn as_key(&self) -> String {
        self.to_string()
    }

    pub fn increment_nonce(&self) -> ReverieNameWithNonce {
        let next_nonce = self.1 + 1;
        ReverieNameWithNonce(
            self.0.clone(),
            next_nonce
        )
    }

    pub fn nonce(&self) -> usize {
        self.1
    }

    pub fn to_reverie_id(&self) -> ReverieIdToNameKey {
        ReverieIdToNameKey::from(self.clone())
    }
}

impl Display for ReverieNameWithNonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.0, self.1)
    }
}

impl Into<String> for ReverieNameWithNonce {
    fn into(self) -> String {
        format!("{}-{}", self.0, self.1)
    }
}

impl From<String> for ReverieNameWithNonce {
    fn from(s: String) -> Self {
        match REVERIE_NAME_NONCE_REGEX.captures(&s).map(|c| c.extract()) {
            Some((_c, [name, nonce])) => {
                ReverieNameWithNonce(name.to_string(), nonce.parse::<usize>().unwrap())
            }
            None => {
                ReverieNameWithNonce("".to_string(), 0)
            }
        }
    }
}
