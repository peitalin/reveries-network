use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};
use color_eyre::eyre::{Result, anyhow};
use std::error::Error as StdError;
use std::fmt;
use std::fs;

use p256::ecdsa::{VerifyingKey, Signature};
use signature::Verifier;
use base64::{Engine, engine::general_purpose::STANDARD as base64_standard};
use llm_proxy::usage::{
    UsageData,
    SignedUsageReport,
    UsageReportPayload,
};
use elliptic_curve::pkcs8::DecodePublicKey;

/// Path to the proxy's public key file
pub const PROXY_PUBLIC_KEY_PATH: &str = "./llm-proxy/shared_identity/proxy_public.pem";

/// Alternative paths to try if the main path fails
pub const ALT_PROXY_KEY_PATHS: [&str; 2] = [
    "/app/llm-proxy/shared_identity/proxy_public.pem", // Docker absolute path
    "/Users/pta/Dev/rust/1Up-network/llm-proxy/shared_identity/proxy_public.pem", // Local dev path
];

#[derive(Debug)]
pub enum VerificationError {
    Base64Payload(base64::DecodeError),
    Base64Signature(base64::DecodeError),
    JsonPayload(serde_json::Error),
    SignatureFormat(signature::Error),
    VerificationFailed,
}

impl fmt::Display for VerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerificationError::Base64Payload(e) => write!(f, "Payload base64 decode error: {}", e),
            VerificationError::Base64Signature(e) => write!(f, "Signature base64 decode error: {}", e),
            VerificationError::JsonPayload(e) => write!(f, "Payload JSON deserialize error: {}", e),
            VerificationError::SignatureFormat(e) => write!(f, "Signature format error: {}", e),
            VerificationError::VerificationFailed => write!(f, "Signature verification failed"),
        }
    }
}

impl StdError for VerificationError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            VerificationError::Base64Payload(e) => Some(e),
            VerificationError::Base64Signature(e) => Some(e),
            VerificationError::JsonPayload(e) => Some(e),
            VerificationError::SignatureFormat(e) => Some(e),
            VerificationError::VerificationFailed => None,
        }
    }
}


pub fn verify_usage_report(
    report: &SignedUsageReport,
    key: &VerifyingKey,
) -> Result<UsageReportPayload, VerificationError> {
    tracing::debug!("Verifying usage report...");

    // 1. Decode Base64 Payload and Signature
    let payload_bytes = base64_standard.decode(&report.payload)
        .map_err(VerificationError::Base64Payload)?;
    let sig_bytes = base64_standard.decode(&report.signature)
        .map_err(VerificationError::Base64Signature)?;

    // 2. Deserialize Payload (for verification and returning)
    let payload_data: UsageReportPayload = serde_json::from_slice(&payload_bytes)
        .map_err(VerificationError::JsonPayload)?;

    // 3. Parse Signature
    let signature = Signature::from_slice(&sig_bytes)
        .map_err(VerificationError::SignatureFormat)?;

    // 4. Verify Signature
    if let Err(e) = key.verify(&payload_bytes, &signature) {
        // Log more details about the verification failure
        warn!("Signature verification failed:");
        warn!("  - Payload (first 100 chars): {:?}", String::from_utf8_lossy(&payload_bytes[..payload_bytes.len().min(100)]));
        warn!("  - Signature: {:?}", signature);
        warn!("  - Key: {:?}", key);
        return Err(VerificationError::VerificationFailed);
    }

    info!("Signature VERIFIED for usage report: Input={}, Output={}",
          payload_data.usage.input_tokens, payload_data.usage.output_tokens);

    // Return the deserialized payload data if verification succeeds
    Ok(payload_data)
}

// Add a function to load the proxy's public key
pub fn load_proxy_key(path: &str) -> Result<VerifyingKey> {
    info!("Loading proxy public key from: {}", path);

    let pem_data = match fs::read_to_string(path) {
        Ok(data) => {
            info!("Successfully read public key file ({} bytes)", data.len());
            data
        },
        Err(e) => {
            error!("Failed to read proxy public key file {}: {}", path, e);
            return Err(anyhow!("Failed to read proxy public key file {}: {}", path, e));
        }
    };

    VerifyingKey::from_public_key_pem(&pem_data)
        .map_err(|e| anyhow!("Failed to parse proxy public key: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{SigningKey, Signature};
    use signature::Signer;
    use rand_core::OsRng;
    use llm_proxy::usage::{UsageData, UsageReportPayload, SignedUsageReport};

    /// Generate a random keypair for testing
    fn generate_test_keypair() -> (SigningKey, VerifyingKey) {
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = *signing_key.verifying_key(); // Dereference to get owned key
        (signing_key, verifying_key)
    }

    /// Create a test usage report and sign it
    fn create_signed_report(signing_key: &SigningKey) -> (UsageReportPayload, SignedUsageReport) {
        // Create test usage data
        let usage = UsageData {
            input_tokens: 100,
            output_tokens: 250,
            cache_creation_input_tokens: Some(50),
            cache_read_input_tokens: None,
        };

        // Create payload
        let payload = UsageReportPayload {
            usage,
            timestamp: 1745468461, // Fixed timestamp for consistent tests
        };

        // Serialize and sign
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let signature: Signature = signing_key.sign(&payload_bytes);

        // Create signed report
        let signed_report = SignedUsageReport {
            payload: base64_standard.encode(&payload_bytes),
            signature: base64_standard.encode(signature.to_bytes()),
        };

        (payload, signed_report)
    }

    #[test]
    fn test_verification_success() {
        // Generate keypair
        let (signing_key, verifying_key) = generate_test_keypair();

        // Create and sign report
        let (expected_payload, signed_report) = create_signed_report(&signing_key);

        // Verify with correct key
        let result = verify_usage_report(&signed_report, &verifying_key);

        // Verification should succeed
        assert!(result.is_ok(), "Verification failed when it should succeed");

        // Verify the returned payload matches expected
        let payload = result.unwrap();
        assert_eq!(payload.usage.input_tokens, expected_payload.usage.input_tokens);
        assert_eq!(payload.usage.output_tokens, expected_payload.usage.output_tokens);
        assert_eq!(payload.timestamp, expected_payload.timestamp);
    }

    #[test]
    fn test_verification_wrong_key() {
        // Generate two keypairs - one for signing, one for (wrong) verification
        let (signing_key, _) = generate_test_keypair();
        let (_, wrong_verifying_key) = generate_test_keypair();

        // Create and sign report with first key
        let (_, signed_report) = create_signed_report(&signing_key);

        // Try to verify with wrong key
        let result = verify_usage_report(&signed_report, &wrong_verifying_key);

        // Verification should fail
        assert!(result.is_err(), "Verification succeeded with wrong key");

        // Check the error type
        match result.unwrap_err() {
            VerificationError::VerificationFailed => {} // Expected error
            other => panic!("Unexpected error type: {:?}", other),
        }
    }

    #[test]
    fn test_verification_tampered_payload() {
        // Generate keypair
        let (signing_key, verifying_key) = generate_test_keypair();

        // Create and sign report
        let (_, mut signed_report) = create_signed_report(&signing_key);

        // Tamper with the payload
        let mut decoded_payload = base64_standard.decode(&signed_report.payload).unwrap();
        if let Some(byte) = decoded_payload.get_mut(10) {
            *byte = byte.wrapping_add(1); // Change one byte
        }
        signed_report.payload = base64_standard.encode(decoded_payload);

        // Try to verify tampered report
        let result = verify_usage_report(&signed_report, &verifying_key);

        // Verification should fail
        assert!(result.is_err(), "Verification succeeded with tampered payload");
    }

    #[test]
    fn test_verification_tampered_signature() {
        // Generate keypair
        let (signing_key, verifying_key) = generate_test_keypair();

        // Create and sign report
        let (_, mut signed_report) = create_signed_report(&signing_key);

        // Tamper with the signature
        let mut decoded_sig = base64_standard.decode(&signed_report.signature).unwrap();
        if let Some(byte) = decoded_sig.get_mut(5) {
            *byte = byte.wrapping_add(1); // Change one byte
        }
        signed_report.signature = base64_standard.encode(decoded_sig);

        // Try to verify with tampered signature
        let result = verify_usage_report(&signed_report, &verifying_key);

        // Verification should fail
        assert!(result.is_err(), "Verification succeeded with tampered signature");
    }
}