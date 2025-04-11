pub mod heartbeat_data;
pub mod peer_info;

use colored::Colorize;
use color_eyre::owo_colors::OwoColorize;
use libp2p::PeerId;
use serde::{Serialize, Deserialize};
use std::collections::{HashMap, HashSet};
use tracing::{warn, info};

use crate::{get_node_name, short_peer_id, types::AgentNameWithNonce};
use crate::types::{
    FragmentNumber,
    VesselStatus,
    ReverieId,
    ReverieCapsulefrag,
    ReverieMessage,
};
use crate::behaviour::heartbeat_behaviour::TeeAttestation;
use peer_info::{PeerInfo, AgentVesselInfo};


#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub(crate) struct TrackReverieFragment {
    pub reverie_id: ReverieId,
    frag_num: usize
}


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
    pub(crate) kfrag_providers: HashMap<ReverieId, HashSet<PeerId>>,
    // Capsule frags for each Agent (encrypted secret key fragments)
    // also contains ciphtertexts atm.
    pub(crate) cfrags: HashMap<ReverieId, ReverieCapsulefrag>,
    pub(crate) reverie_metadata: HashMap<ReverieId, AgentVesselInfo>,
    pub(crate) reverie: HashMap<ReverieId, ReverieMessage>,
    // Tracks which Fragments a Peer holds, so we know which fragments
    // to delete from a peer when a node fails
    pub(crate) peers_to_reverie_frags: HashMap<PeerId, HashSet<TrackReverieFragment>>,
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
            peer_info: HashMap::new(),
            kfrag_providers: HashMap::new(),
            cfrags: HashMap::new(),
            reverie_metadata: HashMap::new(),
            reverie: HashMap::new(),
            peers_to_reverie_frags: HashMap::new(),
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

    pub fn set_peer_info_agent_vessel(&mut self, agent_vessel_info: &AgentVesselInfo) {

        let AgentVesselInfo {
            agent_name_nonce,
            reverie_id,
            total_frags,
            threshold,
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
                    reverie_id: reverie_id.clone(),
                    total_frags: total_frags.clone(),
                    threshold: threshold.clone(),
                    current_vessel_peer_id: current_vessel_peer_id.clone(),
                    next_vessel_peer_id: next_vessel_peer_id.clone(),
                })
            }
        }

        if current_vessel_peer_id == &self.peer_id {
            self.vessel_agent = Some(serde_json::json!({
                "agent_name_nonce": agent_name_nonce.clone().to_string(),
                "total_frags": total_frags.clone(),
                "threshold": threshold.clone(),
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

    /// Confirmed KeyFrag Providers from request-response protocol
    pub fn insert_kfrag_provider(
        &mut self,
        peer_id: PeerId,
        reverie_id: ReverieId,
        frag_num: usize,
    ) {
        // record Peer as Kfrag holder.
        // make it easy to query list of all Peers for a given Reverie
        self.kfrag_providers
            .entry(reverie_id.clone())
            .and_modify(|hset| {
                hset.insert(peer_id);
            })
            .or_insert_with(|| {
                let mut hset = HashSet::new();
                hset.insert(peer_id);
                hset
            });

        // Record which Reverie KeyFragments a Peer holds.
        self.track_reverie_fragment(
            &peer_id,
            TrackReverieFragment {
                reverie_id: reverie_id,
                frag_num: frag_num,
            }
        );
    }

    fn track_reverie_fragment(
        &mut self,
        peer_id: &PeerId,
        track_reverie_fragment: TrackReverieFragment
    ) {
        println!("Tracking reverie_fragment: '{}' frag({})", track_reverie_fragment.reverie_id, track_reverie_fragment.frag_num);
        self.peers_to_reverie_frags
            .entry(*peer_id)
            .and_modify(|hset| {
                hset.insert(track_reverie_fragment.clone());
            })
            .or_insert_with(|| {
                let mut hset = HashSet::new();
                hset.insert(track_reverie_fragment);
                hset
            });
    }

    pub fn remove_kfrag_provider(&mut self, peer_id: &PeerId) {
        // lookup all the agents and fragments Peer holds
        println!("Removing kfrag_provider for {}", short_peer_id(peer_id));
        let opt_track_reverie_fragments = self.peers_to_reverie_frags.get(&peer_id);
        if let Some(track_reverie_fragments) = opt_track_reverie_fragments {
            // remove all kfrag_broadcast_peers entries for the Peer
            for rf in track_reverie_fragments.into_iter() {
                info!("removing frag_num({}): {}", rf.frag_num, rf.reverie_id);
                if let Some(hset) = self.kfrag_providers.get_mut(&rf.reverie_id) {
                    let _removed = hset.remove(peer_id);
                };
                info!("{}: {:?}", "kfrag_providers".blue(), self.kfrag_providers.get(&rf.reverie_id));
            }
            // remove Peer from peers_to_reverie_frags Hashmap
            self.peers_to_reverie_frags.remove(&peer_id);
        } else {
            warn!("{} No entry found.", self.nname());
        }
    }

    pub fn get_kfrag_providers(&self, reverie_id: &ReverieId) -> Option<&HashSet<PeerId>> {
        self.kfrag_providers.get(reverie_id)
    }

    //////////////////////
    //// self.cfrags
    //////////////////////

    pub(crate) fn get_cfrags(&self, reverie_id: &ReverieId) -> Option<&ReverieCapsulefrag> {
        self.cfrags.get(reverie_id)
    }

    pub(crate) fn get_reverie_metadata(&self, reverie_id: &ReverieId) -> Option<&AgentVesselInfo> {
        self.reverie_metadata.get(reverie_id)
    }

    pub(crate) fn get_reverie(&self, reverie_id: &ReverieId) -> Option<&ReverieMessage> {
        self.reverie.get(reverie_id)
    }

    pub(crate) fn insert_cfrags(&mut self, reverie_id: &ReverieId, cfrag: ReverieCapsulefrag) {
        self.cfrags
            .entry(reverie_id.clone())
            .insert_entry(cfrag);
    }

    pub(crate) fn insert_reverie_metadata(&mut self, reverie_id: &ReverieId, agent_metadata: AgentVesselInfo) {
        self.reverie_metadata
            .entry(reverie_id.clone())
            .insert_entry(agent_metadata);
    }

    pub(crate) fn insert_reverie(&mut self, reverie_id: &ReverieId, reverie_message: ReverieMessage) {
        self.reverie
            .entry(reverie_id.clone())
            .insert_entry(reverie_message);
    }

    pub(crate) fn held_cfrags_summary(&self) -> Vec<serde_json::Value> {
        self.cfrags.iter().map(|(reverie_id, cfrag)| {

            let (
                agent_name,
                source_peer_id,
                target_peer_id
            ) = match self.reverie_metadata.get(reverie_id) {
                Some(a) => {
                    (
                        a.agent_name_nonce.clone(),
                        Some(a.current_vessel_peer_id),
                        Some(a.next_vessel_peer_id)
                    )
                },
                None => {
                    (
                        AgentNameWithNonce("NA".to_string(), 0),
                        None,
                        None
                    )
                }
            };

            let cfrag_str: String = format!("{:?}...", cfrag.umbral_capsule_frag.to_vec())[..128].to_string();

            serde_json::json!({
                "reverie_id": reverie_id.to_string(),
                "cfrag": {
                    "agent_name": agent_name,
                    "frag_num": cfrag.frag_num,
                    "threshold": cfrag.threshold,
                    "source_pubkey": cfrag.source_pubkey,
                    "target_pubkey": cfrag.target_pubkey,
                    "source_verifying_pubkey": cfrag.source_verifying_pubkey,
                    "target_verifying_pubkey": cfrag.target_verifying_pubkey,
                    "vessel_peer_id": source_peer_id,
                    "next_vessel_peer_id": target_peer_id,
                    "kfrag_provider_peer_id": cfrag.kfrag_provider_peer_id,
                    "cfrag": cfrag_str,
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
