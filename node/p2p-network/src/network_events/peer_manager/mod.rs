pub mod heartbeat_data;
pub mod peer_info;

use colored::Colorize;
use color_eyre::owo_colors::OwoColorize;
use libp2p::PeerId;
use serde::{Serialize, Deserialize};
use std::collections::{HashMap, HashSet};
use tracing::{warn, info};

use crate::{get_node_name, short_peer_id, types::AgentNameWithNonce};
use crate::types::{CapsuleFragmentMessage, FragmentNumber, VesselStatus};
use crate::behaviour::heartbeat_behaviour::TeeAttestation;
use peer_info::{PeerInfo, AgentVesselInfo};


#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct AgentFragment {
    pub agent_name_nonce: AgentNameWithNonce,
    frag_num: usize
}


// /// Each PRE secret has
// - a unique ID
// - ciphertext
// - peers that hold cfrags
// - decryptor (some peer ID with a pubkey)
// - cfrags (private)


/// Manages Peers and their events
#[derive(Debug)]
pub(crate) struct PeerManager {
    pub(crate) node_name: String,
    pub(crate) peer_id: PeerId,
    pub(crate) vessel_status: VesselStatus,
    // Tracks agent info if node is a vessel
    pub(crate) vessel_agent: Option<serde_json::Value>,
    // Tracks Vessel Nodes
    pub(crate) peer_info: HashMap<PeerId, PeerInfo>,
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

impl PeerManager {
    pub fn new(node_name: String, peer_id: PeerId) -> Self {
        Self {
            node_name: node_name,
            peer_id: peer_id,
            vessel_status: VesselStatus::EmptyVessel,
            vessel_agent: None,
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
        self.peer_info.insert(
            peer_id,
            PeerInfo::new(peer_id, self.avg_window)
        );
    }

    pub fn remove_peer_info(&mut self, peer_id: &PeerId) {
        self.peer_info.remove(peer_id);
    }

    pub fn peer_info_has_agent(&mut self, peer_id: &PeerId) -> bool {
        if let Some(pinfo) = self.peer_info.get(peer_id) {
            pinfo.agent_vessel.is_some()
        } else {
            false
        }
    }

    pub fn set_peer_info_agent_vessel(
        &mut self,
        agent_vessel_info: &AgentVesselInfo,
    ) {
        let AgentVesselInfo {
            agent_name_nonce,
            total_frags,
            current_vessel_peer_id,
            next_vessel_peer_id,
        } = agent_vessel_info;

        info!("Setting vessel for Agent: {}", agent_name_nonce.yellow());
        println!("{}", format!("\tCurrent vessel:\t{}", get_node_name(&current_vessel_peer_id).bright_blue()));
        println!("{}", format!("\tNext vessel:\t{}", get_node_name(&next_vessel_peer_id).bright_blue()));

        match self.peer_info.get_mut(&current_vessel_peer_id) {
            None => {},
            Some(peer_info) => {
                peer_info.agent_vessel = Some(AgentVesselInfo {
                    agent_name_nonce: agent_name_nonce.clone(),
                    total_frags: total_frags.clone(),
                    current_vessel_peer_id: current_vessel_peer_id.clone(),
                    next_vessel_peer_id: next_vessel_peer_id.clone(),
                })
            }
        }

        println!("self.peer_id {}", self.peer_id);
        println!("next_vessel_peer_id {}", next_vessel_peer_id);

        if current_vessel_peer_id == &self.peer_id {
            self.vessel_agent = Some(serde_json::json!({
                "agent_name_nonce": agent_name_nonce.clone().to_string(),
                "total_frags": total_frags.clone(),
                "current_vessel_peer_id": current_vessel_peer_id,
                "current_vessel_node_name": get_node_name(&current_vessel_peer_id),
                "next_vessel_peer_id": next_vessel_peer_id,
                "next_vessel_node_name":  get_node_name(&next_vessel_peer_id),
            }));
        }
    }

    pub fn update_peer_heartbeat(&mut self, peer_id: PeerId, tee_payload: TeeAttestation) {
        match self.peer_info.get_mut(&peer_id) {
            Some(peer_info) => {
                peer_info.heartbeat_data.update(tee_payload);
            }
            None => {
                let mut new_peer_info = PeerInfo::new(peer_id, self.avg_window); // heartbeat_avg_window
                new_peer_info.heartbeat_data.update(tee_payload);
                self.peer_info.insert(peer_id, new_peer_info);
            }
        };
    }

    pub fn make_heartbeat_tee_log(&self, peer_id: PeerId) -> Option<String> {
        if let Some(peer_info) = self.peer_info.get(&peer_id) {
            match &peer_info.heartbeat_data.tee_payload.tee_attestation {
                Some(quote) => {
                    return Some(format!(
                        "{} {} {} {}",
                        format!("{}{}",
                            "Heartbeat from ".bright_black(),
                            get_node_name(&peer_id).magenta(),
                        ),
                        format!("Block({})", peer_info.heartbeat_data.tee_payload.block_height).bright_black(),
                        "TEE Pubkey:".bright_black(),
                        format!("{}", hex::encode(quote.signature.ecdsa_attestation_key)).black(),
                    ))
                    // format!(
                    //     "{} HeartbeatData: Block: {}",
                    //     short_peer_id(peer_id),
                    //     peer_info.heartbeat_data.tee_payload.block_height,
                    // );
                },
                None => {
                    // format!(
                    //     "{} HeartbeatData: Block: {}\t{:?}",
                    //     short_peer_id(peer_id),
                    //     peer_info.heartbeat_data.tee_payload.block_height,
                    //     peer_info.heartbeat_data.tee_payload
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

    pub fn remove_kfrags_broadcast_peer(&mut self, peer_id: &PeerId) {
        // lookup all the agents and fragments Peer holds
        println!("Removing kfrag_broadcast_peers for {}", short_peer_id(peer_id));
        let opt_agent_fragments = self.peers_to_agent_frags.get(&peer_id);

        if let Some(agent_fragments) = opt_agent_fragments {
            // remove all kfrag_broadcast_peers entries for the Peer
            for af in agent_fragments.into_iter() {

                info!("removing frag_num({}): {}", af.frag_num, af.agent_name_nonce);
                if let Some(
                    hmap
                ) = self.kfrag_broadcast_peers.get_mut(&af.agent_name_nonce) {
                    if let Some(hset) = hmap.get_mut(&af.frag_num) {
                        let _removed = hset.remove(peer_id);
                    }
                };

                info!("{}: {:?}", "kfrag_broadcast_peers".blue(),
                    self.kfrag_broadcast_peers.get(&af.agent_name_nonce));
            }
            // remove Peer from peers_to_agent_frags Hashmap
            self.peers_to_agent_frags.remove(&peer_id);
        } else {
            warn!("{} No entry found.", self.nname());
        }
    }

    // Returns kfrag broadcast peers (peers subscribed to kfrag broadcast channels)
    // These are not necesssarily peers who actually hold a kfrag
    // (they could have just joined the network)
    pub fn get_kfrag_broadcast_peers(
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

    pub(crate) fn held_cfrags_summary(&self) -> Vec<serde_json::Value> {
        self.cfrags.iter().map(|(agent_name_nonce, cfrag)| {
            serde_json::json!({
                "agent_name_nonce": agent_name_nonce.to_string(),
                "cfrag": {
                    "frag_num": cfrag.frag_num,
                    "threshold": cfrag.threshold,
                    "alice_pk": cfrag.alice_pk,
                    "bob_pk": cfrag.bob_pk,
                    "verifying_pk": cfrag.verifying_pk,
                    "sender_peer_id": cfrag.sender_peer_id,
                    "vessel_peer_id": cfrag.vessel_peer_id,
                    "next_vessel_peer_id": cfrag.next_vessel_peer_id,
                    "cfrag": format!("{:?}", cfrag.cfrag.to_bytes_simple())[..128],
                }
            })
        }).collect::<>()
    }

    fn nname(&self) -> String {
        format!("{}{}", self.node_name.yellow(), ">".blue())
    }

}

pub trait Punisher {
    fn excommmunicate_peer(&mut self, peer_id: PeerId);
}
