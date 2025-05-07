use std::fs;
use std::path::Path;
use color_eyre::{Result, eyre::anyhow};
use ed25519_dalek::{VerifyingKey as EdDalekVerifyingKey, PUBLIC_KEY_LENGTH};
use pkcs8::{EncodePublicKey, LineEnding};
use tracing::{info, warn, error};

use libp2p::PeerId;
use libp2p::identity;
use libp2p_identity::Keypair as IdentityKeypair;
use libp2p_identity::PublicKey;

use crate::env_var::NODE_SEED_NUM;


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


pub fn export_libp2p_public_key(keypair: &IdentityKeypair, export_path: &str) -> Result<()> {
    let public_key: PublicKey = keypair.public();

    match public_key.try_into_ed25519() {
        Ok(libp2p_ed_pubkey) => {
            info!("Keypair is Ed25519, exporting public key.");

            let pubkey_bytes = libp2p_ed_pubkey.to_bytes();
            let ed_pubkey_dalek = EdDalekVerifyingKey::from_bytes(&pubkey_bytes)
                .map_err(|e| anyhow!("Failed to create ed25519_dalek key from bytes: {}", e))?;

            if let Some(parent_dir) = Path::new(export_path).parent() {
                fs::create_dir_all(parent_dir)
                    .map_err(|e| anyhow!("Failed to create directory {}: {}", parent_dir.display(), e))?;
            }

            let pem_string = ed_pubkey_dalek.to_public_key_pem(LineEnding::LF)
                .map_err(|e| anyhow!("Failed to encode Ed25519 public key to PEM: {}", e))?;

            fs::write(export_path, pem_string)
                .map_err(|e| anyhow!("Failed to write public key PEM to {}: {}", export_path, e))?;

            Ok(())
        }
        Err(_) => {
            Err(anyhow!("Cannot export non-Ed25519 public key to PEM currently."))
        }
    }
}