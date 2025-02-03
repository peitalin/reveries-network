mod heartbeat_data;

use libp2p::PeerId;
use std::collections::{HashMap, HashSet};
use crate::{get_node_name, short_peer_id};
use crate::types::{AgentName, AgentNonce, CapsuleFragmentIndexed, FragmentNumber};

use super::heartbeat_behaviour::heartbeat_handler::TeeAttestation;


#[derive(Clone, Debug)]
pub(crate) struct PeerInfo {
    pub peer_id: PeerId,
    // pub peer_umbral_public_key: Option<umbral_pre::PublicKey>,
    pub peer_heartbeat_data: heartbeat_data::HeartBeatData,
    /// name of the Agent this peer is currently hosting (if any)
    pub agent_vessel: Option<AgentVessel>,
    pub client_version: Option<String>,
}

impl PeerInfo {
    pub fn new(peer_id: PeerId, heartbeat_avg_window: u32) -> Self {
        Self {
            peer_id,
            // peer_umbral_public_key: None,
            peer_heartbeat_data: heartbeat_data::HeartBeatData::new(heartbeat_avg_window),
            agent_vessel: None,
            client_version: None,
        }
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct AgentVessel {
    pub agent_name: String,
    pub agent_nonce: usize,
    pub prev_vessel_peer_id: PeerId,
    pub next_vessel_peer_id: PeerId,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct AgentFragment {
    agent_name: String,
    agent_nonce: usize,
    frag_num: u32
}

/// Manages Peers and their events
#[derive(Debug)]
pub(crate) struct PeerManager {
    // Tracks Vessel Nodes
    pub(crate) peer_info: HashMap<PeerId, PeerInfo>,
    // Tracks which Peers hold which AgentFragments: {agent_name: {frag_num: [PeerId]}}
    pub(crate) kfrags_peers: HashMap<AgentName, HashMap<FragmentNumber, HashSet<PeerId>>>,
    // Capsule frags for each Agent (encrypted secret key fragments)
    pub(crate) cfrags: HashMap<AgentName, CapsuleFragmentIndexed>,
    // Tracks which AgentFragments a specific Peer holds
    pub(crate) peers_to_agent_frags: HashMap<PeerId, HashSet<AgentFragment>>,
    // average heartbeat window for peers (number of entries to track)
    avg_window: u32
}

impl PeerManager {
    pub fn new() -> Self {
        Self {
            kfrags_peers: HashMap::new(),
            peers_to_agent_frags: HashMap::new(),
            peer_info: HashMap::new(),
            cfrags: HashMap::new(),
            avg_window: 10,
        }
    }

    //////////////////////
    //// self.peer_info
    //////////////////////

    pub fn insert_peer_info(&mut self, peer_id: PeerId) {
        self.peer_info
            .insert(peer_id, PeerInfo::new(peer_id, self.avg_window));
    }

    pub fn remove_peer_info(&mut self, peer_id: &PeerId) {
        self.peer_info.remove(peer_id);

    }

    pub fn set_peer_info_agent_vessel(
        &mut self,
        agent_name: &str,
        agent_nonce: usize,
        vessel_peer_id: PeerId,
        next_vessel_peer_id: PeerId,
    ) {
        println!(">>> Setting vessel: {} for Agent: {}", get_node_name(&vessel_peer_id), agent_name);
        match self.peer_info.get_mut(&vessel_peer_id) {
            None => {},
            Some(peer_info) => {
                peer_info.agent_vessel = Some(AgentVessel {
                    agent_name: agent_name.to_string(),
                    agent_nonce: agent_nonce,
                    prev_vessel_peer_id: vessel_peer_id,
                    next_vessel_peer_id: next_vessel_peer_id,
                })
            }
        }
    }

    pub fn get_connected_peers(&self) -> &HashMap<PeerId, PeerInfo> {
        &self.peer_info
    }

    pub fn update_peer_heartbeat(&mut self, peer_id: PeerId, heartbeat_payload: TeeAttestation) {
        match self.peer_info.get_mut(&peer_id) {
            Some(peer_info) => {
                peer_info.peer_heartbeat_data.update(heartbeat_payload);
            }
            None => {
                let mut new_peer_info = PeerInfo::new(peer_id, self.avg_window); // heartbeat_avg_window
                new_peer_info.peer_heartbeat_data.update(heartbeat_payload);
                self.peer_info.insert(peer_id, new_peer_info);
            }
        };
    }

    //////////////////////
    //// self.kfrags_peers
    //////////////////////

    pub fn insert_kfrags_peer(
        &mut self,
        peer_id: PeerId,
        agent_name: String,
        agent_nonce: usize,
        frag_num: u32
    ) {
        let agent_name_nonce_key = format!("{agent_name}-{agent_nonce}");
        // record Peer as Kfrag holder.
        // make it easy to queyr list of all Peers for a given AgentName
        self.kfrags_peers
            .entry(agent_name_nonce_key)
            .and_modify(|hmap| {
                let maybe_hset = hmap.get_mut(&frag_num);
                match maybe_hset {
                    Some(hset) => {
                        hset.insert(peer_id);
                    },
                    None => {
                        let mut hset = HashSet::new();
                        hset.insert(peer_id);
                        hmap.insert(frag_num, hset);
                    }
                }
            })
            .or_insert_with(|| {
                let mut hmap = HashMap::new();
                let mut hset = HashSet::new();
                hset.insert(peer_id);
                hmap.insert(frag_num, hset);
                hmap
            });

        // Record which AgentFragments a Peer holds.
        self.peers_to_agent_frags
            .entry(peer_id)
            .and_modify(|v| {
                v.insert(AgentFragment {
                    agent_name: agent_name,
                    agent_nonce: agent_nonce,
                    frag_num: frag_num
                });
            });

    }

    // removes all peers associated with the agent
    pub fn remove_kfrags_peers_by_agent_name(&mut self, agent_name: &str, agent_nonce: usize) {
        let key = format!("{agent_name}-{agent_nonce}");
        self.kfrags_peers.remove(&key);
    }

    pub fn remove_kfrags_peer(&mut self, peer_id: &PeerId) {
        // lookup which agents and fragments Peer provides, and remove those entries
        println!("\nRemoving kfrags_peers entries for {:?}", short_peer_id(peer_id));
        match self.peers_to_agent_frags.get_mut(&peer_id) {
            None => {
                println!("No entry found....");
            }
            Some(agent_fragments) => {
                // remove all kfrags_peers entries for the Peer
                // println!("peer holds agent_fragments: {:?}", agent_fragments);
                for af in agent_fragments.iter() {
                    if let Some(hmap) = self.kfrags_peers.get_mut(&af.agent_name) {
                        let _removed = hmap.remove(&af.frag_num);
                        // println!("Removed>> {:?}", removed);
                    };
                }
                // remove Peer from peers_to_agent_frags Hashmap
                self.peers_to_agent_frags.remove(&peer_id);
            }
        };
    }

    // Returns all fragment holding peers
    pub fn get_all_kfrag_broadcast_peers(
        &self,
        agent_name: &str,
        agent_nonce: usize
    ) -> Option<&HashMap<u32, HashSet<PeerId>>> {
        let key = format!("{agent_name}-{agent_nonce}");
        self.kfrags_peers.get(&key)
    }

    // Returns peers that just hold a specific fragment
    pub fn get_kfrag_broadcast_peers_by_fragment(
        &self,
        agent_name: &str,
        agent_nonce: &usize,
        frag_num: u32
    ) -> Vec<PeerId> {

        let key = format!("{agent_name}-{agent_nonce}");
        match self.kfrags_peers.get(&key) {
            None => vec![],
            Some(peers_hmap) => {
                match peers_hmap.get(&frag_num) {
                    None => vec![],
                    Some(peers) => {
                        peers.into_iter().map(|p| *p).collect::<Vec<PeerId>>()
                    }
                }
            }
        }
    }

    pub fn insert_peer_agent_fragments(
        &mut self,
        peer_id: &PeerId,
        agent_name: &str,
        agent_nonce: usize,
        frag_num: u32
    ) {
        println!("Adding peer_agent_fragments: '{}-{}' frag({})", agent_name, agent_nonce, frag_num);
        self.peers_to_agent_frags
            .entry(*peer_id)
            .and_modify(|hset| {
                hset.insert(AgentFragment {
                    agent_name: agent_name.to_string(),
                    agent_nonce: agent_nonce,
                    frag_num: frag_num
                });
            })
            .or_insert_with(|| {
                let mut hset = HashSet::new();
                hset.insert(AgentFragment {
                    agent_name: agent_name.to_string(),
                    agent_nonce: agent_nonce,
                    frag_num: frag_num
                });
                hset
            });
    }

    //////////////////////
    //// self.cfrags
    //////////////////////

    pub(crate) fn get_cfrags(
        &self,
        agent_name: &AgentName,
        agent_nonce: &AgentNonce
    ) -> Option<&CapsuleFragmentIndexed> {
        let agent_name_nonce_key = format!("{agent_name}-{agent_nonce}");
        self.cfrags.get(&agent_name_nonce_key)
    }

    pub(crate) fn insert_cfrags(
        &self,
        agent_name: &AgentName,
        agent_nonce: &AgentNonce
    ) -> Option<&CapsuleFragmentIndexed> {
        let agent_name_nonce_key = format!("{agent_name}-{agent_nonce}");
        self.cfrags.get(&agent_name_nonce_key)
    }

}

pub trait Punisher {
    fn excommmunicate_peer(&mut self, peer_id: PeerId);
}
