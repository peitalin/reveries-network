use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::Duration,
};
use color_eyre::Result;
use libp2p::{
    identity,
    kad,
    mdns,
    noise,
    tcp,
    yamux,
    gossipsub,
    StreamProtocol,
};
use tokio::sync::mpsc;

pub use crate::types::UmbralPeerId;
use crate::SendError;
use crate::types::NetworkLoopEvent;
use crate::behaviour::{
    Behaviour,
};
use crate::event_loop::EventLoop;
use crate::event_loop::heartbeat_behaviour::{
    HeartbeatBehaviour,
    HeartbeatConfig
};
use crate::node_client::NodeClient;


/// Creates the network components, namely:
///
/// - The network client to interact with the network layer from anywhere within your application.
/// - The network event stream, e.g. for incoming requests.
/// - The network task driving the network itself.
pub async fn new(secret_key_seed: Option<u8>)
    -> Result<(NodeClient, mpsc::Receiver<NetworkLoopEvent>, EventLoop)> {

    // Create a public/private key pair, either random or based on a seed.
    let (
        peer_id,
        id_keys,
        node_name,
        umbral_key
    ) = generate_peer_keys(secret_key_seed);

    let (heartbeat_failure_sender, heartbeat_failure_receiver) = tokio::sync::mpsc::channel(100);

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default
        )?
        .with_quic()
        .with_behaviour(|key| {

            // To content-address message, we can take the hash of message and use it as an ID.
            let message_id_fn = |message: &gossipsub::Message| {
                let mut s = DefaultHasher::new();
                message.data.hash(&mut s);
                gossipsub::MessageId::from(s.finish().to_string())
            };

            // local peer discovery with mdns
            let mdns = mdns::tokio::Behaviour::new(
                mdns::Config::default(),
                peer_id
            )?;

            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(1))
                .validation_mode(gossipsub::ValidationMode::Strict)
                // This sets the kind of message validation. The default is Strict (enforce message signing)
                // disables duplicate messages from being propagated
                // .message_id_fn(message_id_fn)
                .duplicate_cache_time(Duration::from_secs(5)) // 5 seconds
                // duplicate cache time not working with message_id_fn
                // content-address messages. No two messages of the same content will be propagated.
                .build()
                .map_err(|e| SendError(e.to_string()))?;

            let gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(key.clone()),
                gossipsub_config
            )?;

            Ok(Behaviour {
                kademlia: kad::Behaviour::new(
                    peer_id,
                    kad::store::MemoryStore::new(key.public().to_peer_id())
                ),
                heartbeat: HeartbeatBehaviour::new(
                    // send_timeout should be larger than idle_timeout
                    HeartbeatConfig::new(
                        // Sending of `TeeAttestationBytes` should not take longer than this
                        Duration::from_millis(10_000),
                        // Idle time before sending next `TeeAttestationBytes`
                        Duration::from_millis(8_000),
                        // Max failures allowed. Requests disconnection if reached
                        std::num::NonZeroU32::new(1).unwrap(),
                    ),
                    heartbeat_failure_sender,
                ),
                mdns: mdns,
                gossipsub: gossipsub,
                request_response: libp2p::request_response::cbor::Behaviour::new(
                    [(
                        StreamProtocol::new("/file-exchange/1"),
                        libp2p::request_response::ProtocolSupport::Full,
                    )],
                    libp2p::request_response::Config::default()
                )
            })
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();


    //// Kademlia
    swarm.behaviour_mut()
        .kademlia
        .set_mode(Some(kad::Mode::Server));

    // Listen on all interfaces and whatever port the OS assigns
    // swarm.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse()?)?;
    // swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    let (command_sender, command_receiver) = mpsc::channel(100);
    let (network_events_sender, network_events_receiver) = mpsc::channel(100);
    let (chat_cmd_sender, chat_cmd_receiver) = mpsc::channel(100);

    Ok((
        NodeClient::new(
            peer_id,
            command_sender,
            node_name.clone(),
            chat_cmd_sender,
            umbral_key.clone()
        ),
        network_events_receiver,
        EventLoop::new(
            peer_id,
            swarm,
            command_receiver,
            network_events_sender,
            chat_cmd_receiver,
            node_name,
            umbral_key,
            heartbeat_failure_receiver,
        ),
    ))
}



pub fn generate_peer_keys(secret_key_seed: Option<u8>) -> (
    libp2p::PeerId,
    identity::Keypair,
    String,
    runtime::reencrypt::UmbralKey
) {

    // Create a public/private key pair, either random or based on a seed.
    let (id_keys, node_name, umbral_key) = match secret_key_seed {
        Some(seed) => {

            let mut bytes = [0u8; 32];
            bytes[0] = seed;

            let id_keys = identity::Keypair::ed25519_from_bytes(bytes).unwrap();
            let node_name = crate::make_node_name(seed);
            let umbral_key = runtime::reencrypt::UmbralKey::new(Some(bytes.as_slice()));
            (id_keys, node_name, umbral_key)
        },
        None => {

            let id_keys = identity::Keypair::generate_ed25519();
            let node_name = "Unnamed".to_string();
            let umbral_key = runtime::reencrypt::UmbralKey::new(None);
            (id_keys, node_name, umbral_key)
        }
    };

    let peer_id = id_keys.public().to_peer_id();
    (peer_id, id_keys, node_name, umbral_key)
}
