mod heartbeat_data;

use libp2p::PeerId;
use std::collections::{HashMap, HashSet};
use crate::{get_node_name, short_peer_id, types::AgentNameWithNonce};
use colored::Colorize;
use color_eyre::owo_colors::OwoColorize;
use crate::types::{AgentName, AgentNonce, CapsuleFragmentMessage, FragmentNumber};

use super::heartbeat_behaviour::heartbeat_handler::TeeAttestation;


#[derive(Clone, Debug)]
pub(crate) struct PeerHeartbeatInfo {
    pub peer_id: PeerId,
    // pub peer_umbral_public_key: Option<umbral_pre::PublicKey>,
    pub heartbeat_data: heartbeat_data::HeartBeatData,
    /// name of the Agent this peer is currently hosting (if any)
    pub agent_vessel: Option<AgentVessel>,
    pub client_version: Option<String>,
}

impl PeerHeartbeatInfo {
    pub fn new(peer_id: PeerId, heartbeat_avg_window: u32) -> Self {
        Self {
            peer_id,
            // peer_umbral_public_key: None,
            heartbeat_data: heartbeat_data::HeartBeatData::new(heartbeat_avg_window),
            agent_vessel: None,
            client_version: None,
        }
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct AgentVessel {
    pub agent_name_nonce: AgentNameWithNonce,
    pub total_frags: usize,
    pub prev_vessel_peer_id: PeerId,
    pub next_vessel_peer_id: PeerId,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct AgentFragment {
    pub agent_name_nonce: AgentNameWithNonce,
    frag_num: usize
}

/// Manages Peers and their events
#[derive(Debug)]
pub(crate) struct PeerManager<'a> {
    pub(crate) node_name: &'a str,
    // Tracks Vessel Nodes
    pub(crate) peer_info: HashMap<PeerId, PeerHeartbeatInfo>,
    // Tracks which Peers hold which AgentFragments: {agent_name: {frag_num: [PeerId]}}
    // pub(crate) peers_holding_kfrags: HashMap<AgentName, HashMap<FragmentNumber, HashSet<PeerId>>>,
    // Track which Peers are subscribed to whith AgentFragment broadcast channels:
    pub(crate) kfrag_broadcast_peers: HashMap<AgentNameWithNonce, HashMap<FragmentNumber, HashSet<PeerId>>>,
    // Capsule frags for each Agent (encrypted secret key fragments)
    // also contains ciphtertexts atm.
    // TODO: refactor to move ciphertext to target vessel nodes, away from frag holding nodes
    pub(crate) cfrags: HashMap<AgentNameWithNonce, CapsuleFragmentMessage>,
    // Tracks which AgentFragments a specific Peer holds, so that when a node dies
    // we know which fragments to delete from a peer
    pub(crate) peers_to_agent_frags: HashMap<PeerId, HashSet<AgentFragment>>,
    // average heartbeat window for peers (number of entries to track)
    avg_window: u32
}


impl<'a> PeerManager<'a> {
    pub fn new(node_name: &'a str) -> Self {
        Self {
            node_name,
            kfrag_broadcast_peers: HashMap::new(),
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
            .insert(peer_id, PeerHeartbeatInfo::new(peer_id, self.avg_window));
    }

    pub fn remove_peer_info(&mut self, peer_id: &PeerId) {
        self.peer_info.remove(peer_id);
    }

    pub fn set_peer_info_agent_vessel(
        &mut self,
        agent_name_nonce: &AgentNameWithNonce,
        total_frags: &usize,
        vessel_peer_id: PeerId,
        next_vessel_peer_id: PeerId,
    ) {

        self.log(format!("\nSetting vessel for Agent: {}", agent_name_nonce.yellow()));
        println!("{}", format!("\tCurrent vessel:\t{}", get_node_name(&vessel_peer_id).bright_blue()));
        println!("{}", format!("\tNext vessel:\t{}\n", get_node_name(&next_vessel_peer_id).bright_blue()));

        match self.peer_info.get_mut(&vessel_peer_id) {
            None => {},
            Some(peer_info) => {
                peer_info.agent_vessel = Some(AgentVessel {
                    agent_name_nonce: agent_name_nonce.clone(),
                    total_frags: total_frags.clone(),
                    prev_vessel_peer_id: vessel_peer_id,
                    next_vessel_peer_id: next_vessel_peer_id,
                })
            }
        }
    }

    pub fn get_connected_peers(&self) -> &HashMap<PeerId, PeerHeartbeatInfo> {
        &self.peer_info
    }

    pub fn update_peer_heartbeat(&mut self, peer_id: PeerId, heartbeat_payload: TeeAttestation) {
        match self.peer_info.get_mut(&peer_id) {
            Some(peer_info) => {
                peer_info.heartbeat_data.update(heartbeat_payload);
            }
            None => {
                let mut new_peer_info = PeerHeartbeatInfo::new(peer_id, self.avg_window); // heartbeat_avg_window
                new_peer_info.heartbeat_data.update(heartbeat_payload);
                self.peer_info.insert(peer_id, new_peer_info);
            }
        };
    }

    pub fn log_heartbeat_tee(&self, peer_id: PeerId) -> Option<String> {
        if let Some(peer_info) = self.peer_info.get(&peer_id) {
            match &peer_info.heartbeat_data.payload.tee_attestation {
                Some(quote) => {
                    return Some(format!(
                        "{} {} {} {} {}",
                        short_peer_id(&peer_id).black(),
                        "HeartBeat Block".black(),
                        peer_info.heartbeat_data.payload.block_height.black(),
                        "TEE ESCDA attestation pubkey:".bright_black(),
                        format!("{}", hex::encode(quote.signature.ecdsa_attestation_key)).black(),
                    ))
                    // format!(
                    //     "{} HeartbeatData: Block: {}",
                    //     short_peer_id(peer_id),
                    //     peer_info.heartbeat_data.payload.block_height,
                    // );
                },
                None => {
                    // format!(
                    //     "{} HeartbeatData: Block: {}\n\t{:?}",
                    //     short_peer_id(peer_id),
                    //     peer_info.heartbeat_data.payload.block_height,
                    //     peer_info.heartbeat_data.payload
                    // );
                }
            }
        }
        None
    }

    //////////////////////
    //// self.kfrag_broadcast_peers
    //////////////////////

    pub fn insert_kfrags_broadcast_peer(
        &mut self,
        peer_id: PeerId,
        agent_name_nonce: &AgentNameWithNonce,
        frag_num: usize
    ) {
        // record Peer as Kfrag holder.
        // make it easy to queyr list of all Peers for a given AgentName
        self.kfrag_broadcast_peers
            .entry(agent_name_nonce.clone())
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
                    agent_name_nonce: agent_name_nonce.clone(),
                    frag_num
                });
            })
            .or_insert_with(|| {
                let mut hset = HashSet::new();
                hset.insert(AgentFragment {
                    agent_name_nonce: agent_name_nonce.clone(),
                    frag_num
                });
                hset
            });

    }

    // removes all peers associated with the agent
    pub fn remove_kfrag_broadcast_peers_by_agent(&mut self, agent_name_nonce: &AgentNameWithNonce) {
        self.kfrag_broadcast_peers.remove(agent_name_nonce);
    }

    pub fn remove_kfrags_broadcast_peer(&mut self, peer_id: &PeerId) {

        // lookup all the agents and fragments Peer holds
        println!("\nRemoving kfrag_broadcast_peers for {}", short_peer_id(peer_id));
        let opt_agent_fragments = self.peers_to_agent_frags.get(&peer_id);
        self.log(format!("peer holds agent_fragments: {:?}", opt_agent_fragments));

        if let Some(agent_fragments) = opt_agent_fragments {
            // remove all kfrag_broadcast_peers entries for the Peer
            for af in agent_fragments.into_iter() {

                self.log(format!("removing frag_num({}): {}", af.frag_num, af.agent_name_nonce));
                if let Some(
                    hmap
                ) = self.kfrag_broadcast_peers.get_mut(&af.agent_name_nonce) {
                    if let Some(hset) = hmap.get_mut(&af.frag_num) {
                        let _removed = hset.remove(peer_id);
                    }
                };

                self.log(format!(
                    "{}: {:?}", "kfrag_broadcast_peers".blue(),
                    self.kfrag_broadcast_peers.get(&af.agent_name_nonce)
                ));
            }
            // remove Peer from peers_to_agent_frags Hashmap
            self.peers_to_agent_frags.remove(&peer_id);
        } else {
            self.log("No entry found.");
        }
    }

    // Returns all fragment holding peers
    pub fn get_all_kfrag_peers(
        &self,
        agent_name_nonce: &AgentNameWithNonce,
    ) -> Option<&HashMap<usize, HashSet<PeerId>>> {
        self.kfrag_broadcast_peers.get(agent_name_nonce)
    }

    // Returns peers that just hold a specific fragment
    pub fn get_kfrag_peers_by_fragment(
        &self,
        agent_name_nonce: &AgentNameWithNonce,
        frag_num: usize
    ) -> Vec<PeerId> {
        match self.kfrag_broadcast_peers.get(agent_name_nonce) {
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
        agent_name_nonce: &AgentNameWithNonce,
        frag_num: usize
    ) {
        println!("Adding peer_agent_fragments: '{}' frag({})", agent_name_nonce, frag_num);
        self.peers_to_agent_frags
            .entry(*peer_id)
            .and_modify(|hset| {
                hset.insert(AgentFragment {
                    agent_name_nonce: agent_name_nonce.clone(),
                    frag_num: frag_num
                });
            })
            .or_insert_with(|| {
                let mut hset = HashSet::new();
                hset.insert(AgentFragment {
                    agent_name_nonce: agent_name_nonce.clone(),
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
        agent_name_nonce: &AgentNameWithNonce,
    ) -> Option<&CapsuleFragmentMessage> {
        self.cfrags.get(agent_name_nonce)
    }

    pub(crate) fn insert_cfrags(
        &mut self,
        agent_name_nonce: &AgentNameWithNonce,
        cfrag_indexed: CapsuleFragmentMessage,
    ) {
        self.cfrags
            .entry(agent_name_nonce.clone())
            .insert_entry(cfrag_indexed);
    }

    fn log<S: std::fmt::Display>(&self, message: S) {
        println!("{} {}{} {}",
            "PeerManager".blue(), self.node_name.yellow(), ">".blue(),
            message
        );
    }

}

pub trait Punisher {
    fn excommmunicate_peer(&mut self, peer_id: PeerId);
}
