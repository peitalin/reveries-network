use std::{
    fs,
    fmt, fmt::Display,
    error::Error as StdError,
    path::Path,
};
use p256::ecdsa::{SigningKey, VerifyingKey as P256VerifyingKey};
use rand_core::OsRng;
use rcgen::{KeyPair, CertificateParams, BasicConstraints, Certificate, IsCa, PKCS_ECDSA_P256_SHA256, SanType, Ia5String};
use tracing::*;
use color_eyre::eyre::{Result, anyhow};

// /certs_src is mounted from host into container, and shared with python LLM server
// for Python LLM server to trust llm-proxy as middleman
pub const CA_CERT_PATH: &str = "/certs_src/hudsucker.cer";
// private (unmounted) filepath for Internal API Server TLS and CA_CERT key
pub const CA_KEY_PATH: &str = "/certs_internal/hudsucker.key";
pub const API_SERVER_CERT_PATH: &str = "/certs_internal/internal_api.crt";
pub const API_SERVER_KEY_PATH: &str = "/certs_internal/internal_api.key";


pub fn generate_signing_key() -> Result<(SigningKey, P256VerifyingKey), Box<dyn StdError + Send + Sync>> {
    info!("Generating new ECDSA P-256 key pair.");
    let signing_key = SigningKey::random(&mut OsRng);
    let verifying_key = *signing_key.verifying_key();

    Ok((signing_key, verifying_key))
}

pub fn generate_ca() -> Result<(KeyPair, Certificate), Box<dyn StdError + Send + Sync>> {
    let cert_path = Path::new(CA_CERT_PATH);
    let key_path = Path::new(CA_KEY_PATH);

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

pub fn write_api_server_pem_files(
    ca_cert: &Certificate,
    ca_key_pair: &KeyPair,
) -> Result<()> {
    let cert_path = Path::new(API_SERVER_CERT_PATH);
    let key_path = Path::new(API_SERVER_KEY_PATH);

    if cert_path.exists() && key_path.exists() {
        info!("API server certificate and key files already exist at: {}, {}", API_SERVER_CERT_PATH, API_SERVER_KEY_PATH);
        // TODO: Optionally add validation that the existing cert matches expected SANs/issuer?
        return Ok(());
    }
    info!("Generating new API server PEM certificate and key files, signed by CA...");

    // Ensure parent directory exists
    if let Some(parent) = cert_path.parent() { fs::create_dir_all(parent)?; }
    if let Some(parent) = key_path.parent() { fs::create_dir_all(parent)?; }

    let mut params = CertificateParams::new(vec!["localhost".to_string()])
        .map_err(|e| anyhow!(e))?;
    params.subject_alt_names = vec![
        SanType::DnsName(Ia5String::try_from("localhost".to_string()).map_err(|e| anyhow!(e))?),
        SanType::DnsName(Ia5String::try_from("llm-proxy".to_string()).map_err(|e| anyhow!(e))?),
        SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))),
    ];
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![rcgen::KeyUsagePurpose::KeyEncipherment, rcgen::KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];

    let server_key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .map_err(|e| anyhow!(e))?;

    let server_cert = params.signed_by(&server_key_pair, ca_cert, ca_key_pair)
         .map_err(|e| anyhow!("Failed to sign API server certificate: {}", e))?;

    // Serialize directly to PEM strings
    let server_cert_pem = server_cert.pem();
    let server_key_pem = server_key_pair.serialize_pem();

    // Write PEM strings to files
    fs::write(cert_path, server_cert_pem)
        .map_err(|e| anyhow!("Failed to write API server cert PEM: {}", e))?;
    fs::write(key_path, server_key_pem)
        .map_err(|e| anyhow!("Failed to write API server key PEM: {}", e))?;

    info!("Saved new API server PEM cert and key to: {}, {}", API_SERVER_CERT_PATH, API_SERVER_KEY_PATH);

    Ok(())
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
