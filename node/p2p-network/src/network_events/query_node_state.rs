use crate::{short_peer_id, get_node_name};
use super::NetworkEvents;

impl<'a> NetworkEvents<'a> {
    pub(super) async fn query_node_state(&mut self) -> serde_json::Value {

        let peer_info = self.peer_manager.peer_info
            .iter()
            .map(|(peer_id, peer_info)| {

                let last_hb = peer_info.heartbeat_data.duration_since_last_heartbeat();
                // let avg_hb = peer_info.heartbeat_data.average_time_between_heartbeats();
                let tee_bytes = match &peer_info.heartbeat_data.tee_payload.tee_attestation_bytes {
                    None => "".as_bytes(),
                    Some(bytes) => bytes
                };

                match &peer_info.agent_vessel {
                    None => {
                        serde_json::json!({
                            "peer_id": short_peer_id(peer_id),
                            "node_name": get_node_name(peer_id),
                            "agent_vessel": None as Option<serde_json::Value>,
                            "heartbeat_data": {
                                "last_hb": last_hb,
                                "tee_bytes_len": tee_bytes.len()
                            }
                        })
                    }
                    Some(av) => {
                        serde_json::json!({
                            "peer_id": short_peer_id(peer_id),
                            "node_name": get_node_name(peer_id),
                            "agent_vessel": {
                                "agent_name_nonce": av.agent_name_nonce.to_string(),
                                "next_vessel": get_node_name(&av.next_vessel_peer_id),
                                "current_vessel": get_node_name(&av.current_vessel_peer_id),
                                "total_frags": av.total_frags,
                            },
                            "heartbeat_data": {
                                "last_hb": last_hb,
                                "tee_bytes_len": tee_bytes.len()
                            }
                        })
                    }
                }
            }).collect::<Vec<serde_json::Value>>();

        // self.peer_manager.kfrag_providers
        let kfrag_providers = self.peer_manager.kfrag_providers
            .iter()
            .map(|(agent_name, hmap)| {
                serde_json::json!({
                    "agent_name_nonce": agent_name.to_string(),
                    "kfrag_providers": hmap.iter().map(|(frag_num, peers)| {
                        serde_json::json!({
                            "frag_num": frag_num,
                            "peers": peers.iter()
                                .map(|peer_id| short_peer_id(peer_id))
                                .collect::<Vec<String>>()
                        })
                    }).collect::<Vec<serde_json::Value>>()
                })
            }).collect::<Vec<serde_json::Value>>();

        let agent_in_vessel = &self.peer_manager.vessel_agent;

        let node_state = serde_json::json!({
            "_node_name": self.node_id.node_name,
            "_peer_id": self.node_id.peer_id,
            "_umbral_public_key": self.node_id.umbral_key.public_key,
            "_pending_respawns": self.pending.respawns,
            "_agent_in_vessel": agent_in_vessel,
            "peer_manager": {
                // get all held agent cfrags
                "1_cfrags_summary": self.peer_manager.held_cfrags_summary(),
                // get all kfrag providers
                "2_kfrag_providers": kfrag_providers,
                // peer_info with hb data
                "3_peer_info": peer_info,
            }
        });

        node_state
    }
}