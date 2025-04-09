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
};
use crate::behaviour::heartbeat_behaviour::HeartbeatConfig;
use crate::node_client::NodeCommand;
use crate::types::{
    AgentNameWithNonce,
    FragmentNumber,
    GossipTopic,
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
        prev_agent_name_nonce: AgentNameWithNonce,
    ) {
        // remove pending RespawnIds if need be
        let respawn_id = RespawnId::new(
            &prev_agent_name_nonce,
            &prev_peer_id,
        );
        self.pending.respawns.remove(&respawn_id);

        // remove peer from PeerManager
        self.peer_manager.remove_peer_info(&prev_peer_id);
    }

    pub(crate) async fn handle_peer_heartbeat_failure(&mut self) -> Result<()> {

        let max_time_before_respawn = self.swarm.behaviour()
            .heartbeat
            .config
            .max_time_before_rotation();

        let connected_peers = self.swarm.connected_peers()
            .map(|p| format!("{}", get_node_name(p)))
            .collect::<Vec<String>>();

        // info!("{} connected peers: {:?}", self.nname(), connected_peers);

        let mut failed_agents_placeholder = vec![];
        let peer_info = self.peer_manager.peer_info.clone();

        // iterate and check which peers have stopped sending heartbeats
        for (peer_id, peer_info) in peer_info.iter() {

            let duration = peer_info.heartbeat_data.duration_since_last_heartbeat();
            let node_name = get_node_name(&peer_id).magenta();
            // println!("{}\tlast seen {:.2?} seconds ago", node_name, duration);

            if duration > max_time_before_respawn {

                if let None = &peer_info.agent_vessel {
                    println!("{} failed but wasn't hosting an agent.", node_name);
                    // node will automatically self.remove_peer() after timeout
                }

                if let Some(AgentVesselInfo {
                    agent_name_nonce: prev_agent,
                    total_frags,
                    threshold,
                    next_vessel_peer_id,
                    current_vessel_peer_id
                }) = &peer_info.agent_vessel {

                    let next_agent = AgentNameWithNonce(prev_agent.0.clone(), prev_agent.1 + 1);
                    info!("{}", format!("{} failed. Consensus: voting to reincarnate agent '{}'",
                        node_name,
                        prev_agent
                    ).magenta());

                    // TODO: consensus mechanism to vote for reincarnation
                    failed_agents_placeholder.push(prev_agent);

                    // all peers mark agent as respawning...
                    let respawn_id = RespawnId::new(&prev_agent, &current_vessel_peer_id);
                    self.pending.respawns.insert(respawn_id.clone());

                    ////////////////////////////////////////////////
                    // If this node is the next vessel for the agent
                    if self.node_id.peer_id == *next_vessel_peer_id {

                        // Dispatch a Respawn(agent_name) event to NetworkEvents
                        // - regenerates PRE ciphertexts, and reencryption key frags
                        // - re-broadcasts the new key fragments to peers
                        self.network_event_sender.send(
                            NetworkEvent::RespawnRequest(
                                AgentVesselInfo {
                                    agent_name_nonce: prev_agent.clone(),
                                    total_frags: *total_frags,
                                    threshold: *threshold,
                                    next_vessel_peer_id: *next_vessel_peer_id,
                                    current_vessel_peer_id: current_vessel_peer_id.clone()
                                }
                            )
                        ).await?;

                        info!("{} '{}' {} {}", "Respawning agent".blue(),
                            prev_agent.to_string().green(),
                            "in new vessel:".blue(),
                            self.node_id.node_name.yellow()
                        );

                        // 1) remove Vessel from PeerManagers and Swarm
                        self.remove_peer(&peer_id);

                        // 2) broadcast peers, tell peers to update their PeerManager fields as well:
                        // - kfrags_peers
                        // - peers_to_agent_frags
                        // - peer_info

                    } else {

                        // If node is not the next vessel:
                        // wait for next vessel to re-broadcast cfrags

                        // TODO: once respawn is finished,
                        // 1) new Vessel will tell other nodes to update PeerManager and
                        // 2) update pending.respawns:
                        // self.pending.respawns.remove(&respawn_id_result);

                        if self.pending.respawns.contains(&respawn_id) {
                            info!("Respawn pending: {} -> {}", next_agent, next_vessel_peer_id);
                        } else {
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn handle_internal_heartbeat_failure(&mut self, heartbeat_config: HeartbeatConfig) {
        // Internal heartbeat failure is when this node fails to send heartbeats to external
        // nodes. It realizes it is no longer connected to the network.
        info!("{}", format!("{:?}", heartbeat_config).red());
        info!("{}", "Initiating recovery...".green());
        info!("{}", "Todo: attempting to broadcast/save last good agent_secrets reencryption fragments.".green());
        info!("{}", "Todo: Delete agent secrets, to prevent duplicated agents.".green());

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

        // TODO: Shutdown LLM runtime (if in Vessel Mode), but continue attempting
        // to broadcast the agent_secrets reencryption fragments and ciphertexts.
        //
        // If the node never reconnects to the network, then 1up-network nodes will
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
