use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use color_eyre::Result;
use colored::Colorize;
use futures::StreamExt;
use libp2p::{
    gossipsub::IdentTopic,
    kad,
    identity,
    request_response,
    swarm::Swarm,
    PeerId
};
use tokio::sync::{RwLock, mpsc, oneshot};
use tracing::{info, warn, debug};

use crate::{
    SendError,
    get_node_name,
    short_peer_id,
};
use crate::behaviour::heartbeat_behaviour::HeartbeatConfig;
use crate::node_client::NodeCommand;
use crate::types::{
    ReverieNameWithNonce,
    FragmentNumber,
    NetworkEvent,
    RespawnId,
    VesselPeerId,
    AgentVesselInfo,
    NodeVesselWithStatus,
    VesselStatus,
    SignedVesselStatus,
    ReverieId,
    ReverieIdToAgentName,
};
use crate::node_client::container_manager::{ContainerManager, RestartReason};
use crate::create_network::NODE_SEED_NUM;
use crate::behaviour::Behaviour;
use runtime::reencrypt::UmbralKey;
use super::peer_manager::PeerManager;
use super::NetworkEvents;
use tokio::time;
use time::Duration;


impl<'a> NetworkEvents<'a> {

    pub(crate) fn mark_pending_respawn_complete(
        &mut self,
        prev_peer_id: PeerId,
        prev_agent_name_nonce: ReverieNameWithNonce,
    ) {
        // remove pending RespawnIds if need be
        let respawn_id = RespawnId::new(
            &prev_agent_name_nonce,
            &prev_peer_id,
        );
        self.pending.respawns.remove(&respawn_id);

        // remove peer from PeerManager
        self.remove_peer(&prev_peer_id);
        self.peer_manager.remove_peer_info(&prev_peer_id);
    }

    pub(crate) async fn handle_peer_heartbeat_failure(&mut self) -> Result<()> {

        let max_time_before_respawn = self.swarm.behaviour()
            .heartbeat
            .config
            .max_time_before_rotation();

        let connected_peers: HashSet<&PeerId> = self.swarm.connected_peers().collect();
        let peer_info = self.peer_manager.peer_info.clone();

        // Check if we have peers in peer_info that aren't in connected_peers
        for peer_id in peer_info.keys() {
            if !connected_peers.contains(peer_id) {
                println!("{}", format!("{} exists in peer_info but not in connected_peers", short_peer_id(peer_id)).white());
            }
        }
        // Check if we have connected peers that aren't in peer_info
        for &peer_id in &connected_peers {
            if !peer_info.contains_key(peer_id) {
                println!("{}", format!("{} exists in connected_peers but not in peer_info", short_peer_id(peer_id)).white());
            }
        }
        // let peer_names = connected_peers.iter()
        //     .map(|p| format!("{}", get_node_name(p)))
        //     .collect::<Vec<String>>();
        // info!("{} connected peers: {:?}", self.nname(), connected_peers);

        // Check which peers have stopped sending heartbeats
        for (peer_id, peer_info) in peer_info.iter() {

            let node_name = get_node_name(peer_id);
            // check if peer's heartbeat exceeds max_time_before_respawn

            if self.peer_manager.is_peer_offline(peer_id, max_time_before_respawn, false) {
                info!("{}", format!("{} {} heartbeat failed. Respawn pending.", node_name, peer_id).magenta());
                // Only the next_vessel and kfrag_providers store previous vessel's agent_vessel info
                if let Some(AgentVesselInfo {
                    reverie_id,
                    agent_name_nonce: prev_agent,
                    total_frags,
                    threshold,
                    next_vessel_peer_id,
                    current_vessel_peer_id
                }) = &peer_info.agent_vessel {

                    info!("{}", format!("Reincarnating agent: {}", prev_agent).yellow());
                    // all kfrag_providers mark agent as respawning
                    self.pending.respawns.insert(RespawnId::new(&prev_agent, &current_vessel_peer_id));

                    // If this node is the next vessel for the agent
                    if self.node_id.peer_id == *next_vessel_peer_id {
                        // Dispatch a RespawnRequest event to NetworkEvents
                        info!("Dispatching RespawnRequest for agent {} into new vessel {}",
                            prev_agent.to_string().green(),
                            node_name.green()
                        );
                        // Only the correct next_vessel has the valid signature to get the cfrags and respawn
                        self.network_event_sender.send(
                            NetworkEvent::RespawnRequest(
                                AgentVesselInfo {
                                    reverie_id: reverie_id.clone(),
                                    agent_name_nonce: prev_agent.clone(),
                                    total_frags: *total_frags,
                                    threshold: *threshold,
                                    next_vessel_peer_id: *next_vessel_peer_id,
                                    current_vessel_peer_id: current_vessel_peer_id.clone()
                                }
                            )
                        ).await?;
                        // Only next_vessel removes peer from PeerManagers
                        self.remove_peer(&peer_id);
                        // If node is not the next vessel: wait for next vessel to re-broadcast cfrags
                        // Once respawn is finished:
                        // - New vessel will tell other nodes to update PeerManager and
                        // - update pending.respawns: self.pending.respawns.remove(&respawn_id);
                    }
                } else {
                    // Not a kfrag_provider or next_vessel, safe to remove from PeerManager immediately
                    self.remove_peer(&peer_id);
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn handle_internal_heartbeat_failure(&mut self, heartbeat_config: HeartbeatConfig) {
        // Internal heartbeat failure is when a node fails to send heartbeats to external
        // nodes. It realizes it is no longer connected to the network.
        info!("{}", format!("{:?}", heartbeat_config).red());
        info!("{}", "Initiating recovery...".green());
        info!("{}", "Todo: Delete agent secrets, prevent duplicate agents.".green());

        let peers: Vec<PeerId> = self.swarm.connected_peers().cloned().collect();
        let total_peers = peers.len();
        info!("{}", format!("Disconnecting from {} peers", total_peers).yellow());
        for peer_id in peers {
            self.swarm.disconnect_peer_id(peer_id).ok();
        }

        let time_before_respawn = self.swarm.behaviour()
            .heartbeat
            .config
            .max_time_before_rotation()
            .as_secs();

        // heartbeat protocol failure then triggers ContainerManager to shutdown/reboot container
        for second in 0..time_before_respawn {
            let countdown = time_before_respawn - second;
            tokio::time::sleep(Duration::from_secs(1)).await;
            println!("{}", format!("Rebooting container in {} seconds", countdown).yellow());
        };

        self.container_manager
            .write()
            .await
            .trigger_restart(RestartReason::ScheduledHeartbeatFailure)
            .await.ok();

        // Shutdown LLM runtime (if in Vessel Mode), but continue attempting
        // to broadcast the agent_secrets reencryption fragments and ciphertexts.
        //
        // If the node never reconnects to the network, then nodes will
        // form consensus that the Vessel is dead, and begin reincarnating the Agent
        // from it's last public agent_secret ciphertexts on the Kademlia network.
        //
        // In this case, the node should also delete it's Agent secrets to prevent
        // duplicate instances of an agent running simultaneously
    }

    pub(crate) async fn simulate_heartbeat_failure(&mut self) {
        self.swarm.behaviour_mut()
            .heartbeat
            .trigger_heartbeat_failure().await;
    }

}
