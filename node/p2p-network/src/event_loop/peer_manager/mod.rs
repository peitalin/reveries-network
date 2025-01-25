mod heartbeat_data;

use libp2p::{
    Multiaddr,
    PeerId,
};
use std::collections::{
    HashMap,
    HashSet,
};
use crate::{AgentName, FragmentNumber};

use super::heartbeat::heartbeat_handler::TeeAttestation;


#[derive(Clone, Debug)]
pub(crate) struct PeerInfo {
    pub peer_addresses: HashSet<Multiaddr>,
    pub peer_umbral_public_keys: HashMap<PeerId, umbral_pre::PublicKey>,
    pub peer_heartbeat_data: heartbeat_data::HeartBeatData,
    pub client_version: Option<String>,
}

impl PeerInfo {
    pub fn new(heartbeat_avg_window: u32) -> Self {
        Self {
            peer_addresses: HashSet::new(),
            peer_umbral_public_keys: HashMap::new(),
            peer_heartbeat_data: heartbeat_data::HeartBeatData::new(heartbeat_avg_window),
            client_version: None,
        }
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct AgentFragment {
    agent_name: String,
    frag_num: u32
}


/// Manages Peers and their events
#[derive(Debug)]
pub struct PeerManager {

    // Tracks which Peers hold which AgentFragments
    // { agent_name: { frag_num: [PeerId] }}
    kfrags_peers: HashMap<AgentName, HashMap<FragmentNumber, HashSet<PeerId>>>,

    // Tracks which AgentFragments a specific Peer holds
    peers_to_agent_frags: HashMap<PeerId, HashSet<AgentFragment>>,

    // Tracks Vessel Nodes
    pub vessel_nodes: HashMap<PeerId, PeerInfo>,
}

impl PeerManager {

    pub fn new(
    ) -> Self {
        Self {
            kfrags_peers: HashMap::new(),
            peers_to_agent_frags: HashMap::new(),
            vessel_nodes: HashMap::new(),
        }
    }

    pub fn update_heartbeat(&mut self, peer_id: PeerId, heartbeat_payload: TeeAttestation) {
        match self.vessel_nodes.get_mut(&peer_id) {
            Some(peer_info) => {
                peer_info.peer_heartbeat_data.update(heartbeat_payload);
            }
            None => {
                let avg_window = 10;
                let mut new_peer_info = PeerInfo::new(avg_window); // heartbeat_avg_window
                new_peer_info.peer_heartbeat_data.update(heartbeat_payload);
                self.vessel_nodes.insert(peer_id, new_peer_info);
            }
        };
    }

    // pub fn increment_heartbeat_block_height(&mut self, peer_id: PeerId) {
    //     match self.vessel_nodes.get_mut(&peer_id) {
    //         Some(peer_info) => {
    //             peer_info.heartbeat_data.increment_block_height();
    //         }
    //         None => {}
    //     };
    // }

    pub fn record_umbral_kfrag_peer(&mut self, peer_id: PeerId, agent_name: String, frag_num: u32) {

        // record Peer as Kfrag holder.
        // make it easy to queyr list of all Peers for a given AgentName
        self.kfrags_peers
            .entry(agent_name.clone())
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
                    frag_num: frag_num
                });
            });

    }

    pub fn remove_umbral_kfrag_peer(&mut self, peer_id: PeerId) {
        match self.peers_to_agent_frags.get_mut(&peer_id) {
            None => {}
            Some(agent_fragments) => {
                // remve all kfrags_peers entries for the Peer
                for af in agent_fragments.iter() {
                    if let Some(hmap) = self.kfrags_peers.get_mut(&af.agent_name) {
                        hmap.remove(&af.frag_num);
                    };
                }
                // remove Peer from peers_to_agent_frags Hashmap
                self.peers_to_agent_frags.remove(&peer_id);
            }
        };
    }

    pub fn get_umbral_kfrag_providers(&self, agent_name: &str) -> Option<&HashMap<FragmentNumber, HashSet<PeerId>>> {
        self.kfrags_peers.get(agent_name)
    }

}


fn update_peer_heartbeat(
    peers: &mut HashMap<PeerId, PeerInfo>,
    peer_id: &PeerId,
    block_height: TeeAttestation,
) {
    if let Some(peer) = peers.get_mut(peer_id) {
        peer.peer_heartbeat_data.update(block_height);
    } else {
    }
}

pub trait Punisher {
    fn excommmunicate_peer(&mut self, peer_id: PeerId);
}
