use std::{
    fs,
    fmt, fmt::Display,
    error::Error as StdError,
    path::Path,
};
use ecdsa::elliptic_curve::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use p256::ecdsa::SigningKey;
use rand_core::OsRng;
use rcgen::{KeyPair, CertificateParams, BasicConstraints, Certificate, IsCa};
use tracing::*;

const CA_CERT_PATH: &str = "/certs_src/hudsucker.cer";
const CA_KEY_PATH: &str = "/certs_src/hudsucker.key";
const PRIVATE_KEY_PATH: &str = "/data/proxy_private.key";
const PUBLIC_KEY_PATH: &str = "/shared_identity/proxy_public.pem";


pub fn load_or_generate_signing_key() -> Result<SigningKey, Box<dyn StdError + Send + Sync>> {
    let private_key_path = Path::new(PRIVATE_KEY_PATH);
    let public_key_path = Path::new(PUBLIC_KEY_PATH);

    info!("Generating new ECDSA P-256 key pair.");
    let signing_key = SigningKey::random(&mut OsRng);

    if let Some(parent) = private_key_path.parent() { fs::create_dir_all(parent)?; }
    if let Some(parent) = public_key_path.parent() { fs::create_dir_all(parent)?; }

    let private_key_pem = signing_key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(UsageKeypairError::from)?;

    let public_key_pem = signing_key.verifying_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(UsageKeypairError::from)?;

    fs::write(private_key_path, private_key_pem.as_bytes()).map_err(UsageKeypairError::from)?;
    fs::write(public_key_path, public_key_pem.as_bytes()).map_err(UsageKeypairError::from)?;

    info!("Saved new private key to: {}", PRIVATE_KEY_PATH);
    info!("Saved new public key to: {}", PUBLIC_KEY_PATH);
    Ok(signing_key)
}

pub fn load_or_generate_ca() -> Result<(KeyPair, Certificate), Box<dyn StdError + Send + Sync>> {
    let cert_path = Path::new(CA_CERT_PATH);
    let key_path = Path::new(CA_KEY_PATH);

    if cert_path.exists() && key_path.exists() {
        info!("Loading existing CA certificate and key from files: {}, {}", CA_CERT_PATH, CA_KEY_PATH);
        let ca_cert_pem = fs::read_to_string(cert_path)
            .map_err(|e| format!("Failed to read CA cert file {}: {}", CA_CERT_PATH, e))?;
        let ca_key_pem = fs::read_to_string(key_path)
            .map_err(|e| format!("Failed to read CA key file {}: {}", CA_KEY_PATH, e))?;
        let ca_key_pair = KeyPair::from_pem(&ca_key_pem)
            .map_err(|e| format!("Failed to parse CA private key: {}", e))?;
        let ca_params = CertificateParams::from_ca_cert_pem(&ca_cert_pem)
            .map_err(|e| format!("Failed to parse CA certificate parameters: {}", e))?;
        // Note: self_signed re-signs, but it's okay for loading params.
        let ca_cert = ca_params.self_signed(&ca_key_pair)
            .map_err(|e| format!("Failed to process CA certificate: {}", e))?;
        Ok((ca_key_pair, ca_cert))
    } else {
        info!("Generating new CA certificate and key at: {}, {}", CA_CERT_PATH, CA_KEY_PATH);
        // Ensure parent directory exists
        if let Some(parent) = cert_path.parent() { fs::create_dir_all(parent)?; }
        if let Some(parent) = key_path.parent() { fs::create_dir_all(parent)?; }


        let mut params = CertificateParams::new(vec!["LLM Proxy Generated CA".to_string()])?;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0)); // Set as CA
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        // Generate an ECDSA P-256 key pair explicitly
        let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;

        // Create the certificate using the params and the generated key pair
        let ca_cert = params.self_signed(&key_pair)
             .map_err(|e| format!("Failed to self-sign CA certificate: {}", e))?;

        // Save the generated key and certificate
        let key_pem = key_pair.serialize_pem(); // Serialize the generated key pair
        let cert_pem = ca_cert.pem();

        fs::write(key_path, key_pem)
            .map_err(|e| format!("Failed to write CA key: {}", e))?;

        fs::write(cert_path, cert_pem)
            .map_err(|e| format!("Failed to write CA certificate: {}", e))?;

        info!("Saved new CA key and certificate.");
        Ok((key_pair, ca_cert))
    }
}

#[derive(Debug)]
pub enum UsageKeypairError {
    PrivateKeyEncoding(String),
    PublicKeyEncoding(String),
    FileError(String),
}

impl StdError for UsageKeypairError {}

impl Display for UsageKeypairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PrivateKeyEncoding(e) => write!(f, "Failed to encode private key to PKCS8 PEM: {}", e),
            Self::PublicKeyEncoding(e) => write!(f, "Failed to encode public key to PKCS8 PEM: {}", e),
            Self::FileError(e) => write!(f, "Failed to write private key to file: {}", e),
        }
    }
}

impl From<ecdsa::elliptic_curve::pkcs8::Error> for UsageKeypairError {
    fn from(e: ecdsa::elliptic_curve::pkcs8::Error) -> Self {
        Self::PrivateKeyEncoding(e.to_string())
    }
}

impl From<p256::pkcs8::spki::Error> for UsageKeypairError {
    fn from(e: p256::pkcs8::spki::Error) -> Self {
        Self::PublicKeyEncoding(e.to_string())
    }
}

impl From<std::io::Error> for UsageKeypairError {
    fn from(e: std::io::Error) -> Self {
        Self::FileError(e.to_string())
    }
}
