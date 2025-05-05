use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::Duration,
};
use color_eyre::Result;
use color_eyre::eyre::anyhow;
use libp2p::{
    dns,
    gossipsub,
    identity,
    kad,
    multiaddr::Multiaddr,
    noise,
    tcp,
    yamux,
    PeerId,
    StreamProtocol,
};
use tokio::sync::{mpsc, RwLock};
use std::sync::Arc;
use std::fs;
use std::path::Path;
use ed25519_dalek::{VerifyingKey as EdVerifyingKey, PUBLIC_KEY_LENGTH};
use pkcs8::{EncodePublicKey, LineEnding};
use tracing::{info, warn, error};
use libp2p_identity::Keypair as IdentityKeypair;
use libp2p_identity::PublicKey;
use std::env;

use crate::SendError;
use crate::types::NetworkEvent;
use crate::behaviour::Behaviour;
use crate::behaviour::heartbeat_behaviour::{
    HeartbeatBehaviour,
    HeartbeatConfig
};
use crate::network_events::{NetworkEvents, NodeIdentity};
use crate::node_client::{NodeClient, ContainerManager};
use crate::usage_db::init_usage_db;
use crate::node_client::usage_verification::load_proxy_key;
use crate::env_var::EnvVars;
use p256::ecdsa::VerifyingKey as P256VerifyingKey;
use ed25519_dalek::VerifyingKey as EdDalekVerifyingKey;

thread_local! {
    pub static NODE_SEED_NUM: std::cell::RefCell<usize> = std::cell::RefCell::new(1);
}

/// Creates the network components, namely:
/// - The network client to interact with the network layer from anywhere within your application.
/// - The network event stream, e.g. for incoming requests.
/// - The network task driving the network itself.
pub async fn new(
    secret_key_seed: Option<usize>,
    bootstrap_nodes: Vec<(String, Multiaddr)>,
) -> Result<(
    NodeClient,
    mpsc::Receiver<NetworkEvent>,
    NetworkEvents
)> {
    // Create a public/private key pair, either random or based on a seed.
    let (
        peer_id,
        id_keys,
        node_name,
        umbral_key
    ) = generate_peer_keys(secret_key_seed);

    let test_env = env::var("TEST_ENV").unwrap_or_default().to_lowercase() == "true";
    let export_path  = if test_env {
        let node_test_pubkey_path = String::from("./llm-proxy/pubkeys/p2p-node/p2p_node_test.pub.pem");
        info!("TEST_ENV env var set, using test node public key path: {}", node_test_pubkey_path);
        node_test_pubkey_path
    } else {
        let node_pubkey_path = String::from("./llm-proxy/pubkeys/p2p-node/p2p_node.pub.pem");
        node_pubkey_path
    };

    if let Err(e) = export_libp2p_public_key(&id_keys, &export_path) {
        error!("Failed to export p2p-node public key to {}: {}. Signature verification by proxy will fail.", export_path, e);
    } else {
        info!("Successfully exported p2p-node public key to {}", export_path);
    }

    // TODO: used for determining which fragment the peer subscribes
    // Replace with NODE_SEED_NUM
    let seed = secret_key_seed.unwrap_or(0);

    // Channels
    let (heartbeat_failure_sender, heartbeat_failure_receiver) = tokio::sync::mpsc::channel(100);
    let (heartbeat_sender, heartbeat_receiver) = async_channel::bounded(100);
    let (command_sender, command_receiver) = mpsc::channel(100);
    let (network_events_sender, network_events_receiver) = mpsc::channel(100);

    // Swarm Setup
    let swarm = libp2p::SwarmBuilder::with_existing_identity(id_keys.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default
        )?
        // QUIC has it's own connection timeout.
        .with_quic()
        .with_dns()?
        .with_behaviour(|key| {

            // Configure Kademlia for peer discovery
            let mut kademlia = kad::Behaviour::new(
                peer_id,
                kad::store::MemoryStore::new(key.public().to_peer_id())
            );

            // Enable Kademlia record publishing
            kademlia.set_mode(Some(kad::Mode::Server));

            // Add bootstrap nodes to Kademlia and attempt DNS resolution
            for (peer_id_str, addr) in &bootstrap_nodes {
                if let Ok(bootstrap_peer_id) = peer_id_str.parse::<PeerId>() {
                    kademlia.add_address(&bootstrap_peer_id, addr.clone());
                    // Try to bootstrap immediately
                    if let Err(e) = kademlia.bootstrap() {
                        tracing::warn!("Failed to bootstrap Kademlia: {}", e);
                    }
                }
            }

            // Create identify behavior
            let identify = libp2p_identify::Behaviour::new(
                libp2p_identify::Config::new(
                "/my-node/1.0.0".to_string(),
                key.public(),
                )
            );

            Ok(Behaviour {
                kademlia,
                heartbeat: HeartbeatBehaviour::new(
                    // send_timeout should be larger than idle_timeout
                    HeartbeatConfig {
                        // Sending of `TeeAttestationBytes` should not take longer than this
                        // This is the delay before ContainerManager reboots node.
                        send_timeout: Duration::from_millis(12_000),
                        // Idle time before sending next `TeeAttestationBytes`
                        // This is the delay before Vessels attempt to reincarnate a unresponsive vessel
                        // In production, set this much higher
                        idle_timeout: Duration::from_millis(6_000),
                        // Max failures allowed. Requests disconnection if reached
                        max_failures: 1,
                    },
                    heartbeat_failure_sender,
                    heartbeat_sender,
                ),
                // gossipsub: gossipsub,
                identify: identify,
                request_response: libp2p::request_response::cbor::Behaviour::new(
                    [(
                        StreamProtocol::new("/1up-kfrags-reqres/1.0.0"),
                        libp2p::request_response::ProtocolSupport::Full,
                    )],
                    libp2p::request_response::Config::default()
                )
            })
        })?
        .with_swarm_config(|c|
            c.with_idle_connection_timeout(Duration::from_secs(u64::MAX))
        )
        .build();

    let container_manager = Arc::new(RwLock::new(
        ContainerManager::new(
            std::time::Duration::from_secs(30),
        )
    ));

    let node_identity = NodeIdentity::new(
        node_name.to_string(),
        peer_id,
        id_keys,
        seed,
        umbral_key.clone(),
    );

    let env_vars = EnvVars::load();

    // Initialize Usage Report DB Pool
    let usage_db_pool = init_usage_db()?;
    let proxy_public_key: Option<P256VerifyingKey> = load_proxy_key(&env_vars.PROXY_PUBLIC_KEY_PATH, false).await.ok();

    let node_client = NodeClient::new(
        node_identity.clone(),
        command_sender,
        umbral_key,
        container_manager.clone(),
        heartbeat_receiver,
        proxy_public_key, // Pass loaded key
        usage_db_pool,    // Pass DB pool
    );

    Ok((
        node_client,
        network_events_receiver,
        NetworkEvents::new(
            swarm,
            node_identity,
            command_receiver,
            network_events_sender,
            heartbeat_failure_receiver,
            container_manager
        ),
    ))
}

pub fn generate_peer_keys<'a>(secret_key_seed: Option<usize>) -> (
    libp2p::PeerId,
    identity::Keypair,
    &'a str,
    runtime::reencrypt::UmbralKey
) {

    // Create a public/private key pair, either random or based on a seed.
    let (id_keys, umbral_key) = match secret_key_seed {
        Some(seed) => {
            let mut bytes = [0u8; 32];
            bytes[0] = seed as u8;

            // set seed for working out frag_num this this peer
            NODE_SEED_NUM.with(|n| {
                *n.borrow_mut() = seed;
            });

            let id_keys = identity::Keypair::ed25519_from_bytes(bytes).unwrap();
            let umbral_key = runtime::reencrypt::UmbralKey::new(Some(bytes.as_slice()));
            (id_keys, umbral_key)
        },
        None => {
            let id_keys = identity::Keypair::generate_ed25519();
            let umbral_key = runtime::reencrypt::UmbralKey::new(None);
            (id_keys, umbral_key)
        }
    };

    let peer_id = id_keys.public().to_peer_id();
    let node_name = crate::get_node_name(&peer_id);
    (peer_id, id_keys, node_name, umbral_key)
}

// Function to export the libp2p public key to PEM format
// Make this function public for use in tests
pub fn export_libp2p_public_key(keypair: &IdentityKeypair, export_path: &str) -> Result<()> {
    let public_key: PublicKey = keypair.public();

    match public_key.try_into_ed25519() {
        Ok(libp2p_ed_pubkey) => {
            info!("Keypair is Ed25519, exporting public key.");

            // Get raw bytes from libp2p public key
            let pubkey_bytes = libp2p_ed_pubkey.to_bytes();

            // Construct ed25519_dalek VerifyingKey from bytes
            let ed_pubkey_dalek = EdDalekVerifyingKey::from_bytes(&pubkey_bytes)
                .map_err(|e| anyhow!("Failed to create ed25519_dalek key from bytes: {}", e))?;

            // Ensure the target directory exists
            if let Some(parent_dir) = Path::new(export_path).parent() {
                fs::create_dir_all(parent_dir)
                    .map_err(|e| anyhow!("Failed to create directory {}: {}", parent_dir.display(), e))?;
            }

            // Encode the ed25519_dalek public key to PEM using pkcs8
            let pem_string = ed_pubkey_dalek.to_public_key_pem(LineEnding::LF)
                .map_err(|e| anyhow!("Failed to encode Ed25519 public key to PEM: {}", e))?;

            // Write the PEM string to the file
            fs::write(export_path, pem_string)
                .map_err(|e| anyhow!("Failed to write public key PEM to {}: {}", export_path, e))?;

            Ok(())
        }
        Err(_) => {
            Err(anyhow!("Cannot export non-Ed25519 public key to PEM currently."))
        }
    }
}
