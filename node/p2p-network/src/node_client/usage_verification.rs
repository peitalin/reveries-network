use base64::{Engine, engine::general_purpose::STANDARD as base64_standard};
use color_eyre::eyre::{Result, anyhow};
use colored::Colorize;
use dcap_rs::types::VerifiedOutput;
use elliptic_curve::pkcs8::DecodePublicKey;
use p256::ecdsa::{VerifyingKey, Signature};
use std::{
    error::Error as StdError,
    fmt,
    fs,
    path::Path,
    time::Duration,
};
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use signature::Verifier;
use sha2::{Sha256, Digest};
use tracing::{info, warn, error, debug};
use tokio::time::sleep as tokio_sleep;

use llm_proxy::usage::{
    UsageData,
    SignedUsageReport,
    UsageReportPayload,
};
use runtime::tee_attestation::{self, QuoteV4, QuoteBody, TcbStatus};
use runtime::llm::MCPToolUsageMetrics;
use crate::usage_db::{UsageDbPool, store_usage_payload, read_usage_data_for_reverie};
use super::{NodeClient, NodeCommand};


#[derive(Debug)]
pub enum VerificationError {
    Base64Payload(base64::DecodeError),
    Base64Signature(base64::DecodeError),
    JsonPayload(serde_json::Error),
    SignatureFormat(signature::Error),
    VerificationFailed,
    DcapTcbStatusNotOk(String),
    Base64TdxQuote(base64::DecodeError),
    TdxQuoteBindingMismatch,
    DcapCollateralLoadError(String),
    DcapVerificationFailed(String),
}

impl fmt::Display for VerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerificationError::Base64Payload(e) => write!(f, "Payload base64 decode error: {}", e),
            VerificationError::Base64Signature(e) => write!(f, "Signature base64 decode error: {}", e),
            VerificationError::JsonPayload(e) => write!(f, "Payload JSON deserialize error: {}", e),
            VerificationError::SignatureFormat(e) => write!(f, "Signature format error: {}", e),
            VerificationError::VerificationFailed => write!(f, "Signature verification failed"),
            VerificationError::DcapTcbStatusNotOk(status) => write!(f, "DCAP TCB status check failed: {}.", status),
            VerificationError::Base64TdxQuote(e) => write!(f, "TDX Quote base64 decode error: {}", e),
            VerificationError::TdxQuoteBindingMismatch => write!(f, "TDX Quote report_data mismatch"),
            VerificationError::DcapCollateralLoadError(e) => write!(f, "DCAP collateral load error: {}", e),
            VerificationError::DcapVerificationFailed(e) => write!(f, "DCAP verification failed: {}", e),
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
            VerificationError::DcapTcbStatusNotOk(_) => None,
            VerificationError::Base64TdxQuote(e) => Some(e),
            VerificationError::TdxQuoteBindingMismatch => None,
            VerificationError::DcapCollateralLoadError(_) => None,
            VerificationError::DcapVerificationFailed(_) => None,
        }
    }
}

impl From<runtime::tee_attestation::DcapError> for VerificationError {
    fn from(e: runtime::tee_attestation::DcapError) -> Self {
        match e {
            runtime::tee_attestation::DcapError::CollateralLoadError(s) => VerificationError::DcapCollateralLoadError(s),
            runtime::tee_attestation::DcapError::VerificationPanic(s) => VerificationError::DcapVerificationFailed(format!("Panic: {}", s)), // Reuse DcapVerificationFailed
            runtime::tee_attestation::DcapError::VerificationFailed(s) => VerificationError::DcapVerificationFailed(s),
            runtime::tee_attestation::DcapError::TcbStatusNotOk(s) => VerificationError::DcapTcbStatusNotOk(s),
        }
    }
}


impl NodeClient {
    /// Verifies and potentially stores a usage report received from a proxy.
    /// TODO: move to network_events and dispatch on-chain
    pub async fn report_usage(&mut self, signed_usage_report: SignedUsageReport) -> Result<String> {
        info!("Received usage report: Payload(first 50)={:?}, Signature(first 10)={:?}",
              &signed_usage_report.payload[..signed_usage_report.payload.len().min(50)],
              &signed_usage_report.signature[..signed_usage_report.signature.len().min(10)]);

        // Get the public key from self.proxy_public_key
        let proxy_public_key = self.llm_proxy_public_key.read().await;
        let opt_public_key = proxy_public_key.as_ref();
        let public_key = match opt_public_key {
            Some(ref key) => key,
            None => {
                error!("NodeClient: LLM Proxy public key not available for usage report verification. Has it registered yet?");
                return Err(anyhow!("LLM Proxy public key not available. Cannot verify usage report."));
            }
        };

        match verify_usage_report(&signed_usage_report, public_key) { // Pass the borrowed key
            Ok(payload) => {
                info!("NodeClient: Usage report verified successfully.");
                // Verification successful, now store the payload
                if let Err(db_err) = store_usage_payload(&self.usage_db_pool, &payload) {
                    error!("NodeClient: Failed to store verified usage payload in DB: {}", db_err);
                }
                Ok("Usage report verified.".to_string())
            }
            Err(verification_error) => {
                error!("NodeClient: Usage report verification failed: {}", verification_error);
                Err(anyhow!("Usage report verification failed: {}", verification_error))
            }
        }
    }

    pub fn read_usage_data_for_reverie(&self, reverie_id: &str) -> Result<MCPToolUsageMetrics> {

        let mut metrics = MCPToolUsageMetrics::default();
        // Read usage data from database for this reverie (after LLM calls)
        info!("About to read usage data for reverie: {}", reverie_id);
        match read_usage_data_for_reverie(&self.usage_db_pool, &reverie_id) {
            Ok(usage_records) => {
                info!("Found {} usage records for reverie {}", usage_records.len(), reverie_id);

                // Convert usage records to MCPToolUsageMetrics format
                for record in usage_records {
                    info!("Processing usage record: {}", record.request_id);
                    let usage_record = runtime::llm::UsageRecord {
                        request_id: record.request_id,
                        timestamp: record.timestamp,
                        input_tokens: record.usage.input_tokens,
                        output_tokens: record.usage.output_tokens,
                        cache_creation_tokens: record.usage.cache_creation_input_tokens,
                        cache_read_tokens: record.usage.cache_read_input_tokens,
                        tool_name: record.usage.tool_use.as_ref().map(|tu| tu.name.clone()),
                        tool_type: record.usage.tool_use.as_ref().map(|tu| tu.tool_type.clone()),
                        linked_tool_id: record.linked_tool_use_id,
                        reverie_id: record.usage.reverie_id,
                        spender_address: record.usage.spender,
                        spender_type: record.usage.spender_type,
                    };
                    metrics.add_usage_record(usage_record);
                }
            },
            Err(e) => {
                error!("Failed to read usage data from database for reverie {}: {}", reverie_id, e);
            }
        }

        let usage_report = metrics.generate_report();
        println!("{}", usage_report);

        Ok(metrics)
    }
}

pub fn verify_usage_report(
    report: &SignedUsageReport,
    key: &VerifyingKey,
) -> Result<UsageReportPayload, VerificationError> {
    tracing::debug!("Verifying usage report...");

    ////////////////////////////////////////
    ///// ECDSA Signature Verification /////
    ////////////////////////////////////////
    debug!("Verifying ECDSA signature...");
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

    // 4. Verify ECDSA Signature
    key.verify(&payload_bytes, &signature)
        .map_err(|_| VerificationError::VerificationFailed)?;
    info!("{}", "ECDSA Signature VERIFIED.".green());

    ////////////////////////////////////////
    ///// TDX Quote Verification /////
    ////////////////////////////////////////
    debug!("Verifying TDX Quote binding...");
    // 5. Decode Base64 TDX Quote
    let tdx_quote_bytes = base64_standard.decode(&report.tdx_quote)
        .map_err(VerificationError::Base64TdxQuote)?;

    // 6. Parse TDX Quote bytes into QuoteV4 struct
    let quote = QuoteV4::from_bytes(&tdx_quote_bytes);

    // 7. Recalculate Payload Hash using the shared function
    let calculated_report_data = runtime::tee_attestation::hash_payload_for_tdx_report_data(&payload_bytes);

    // --- Only perform binding check if compiled for TDX ---
    // 8. Compare Calculated Hash (Padded) with Quote's Report Data
    #[cfg(all(target_os = "linux", feature = "tdx_enabled"))]
    {
        let quote_report_data: &[u8; 64] = match &quote.quote_body {
            QuoteBody::SGXQuoteBody(report) => &report.report_data,
            QuoteBody::TD10QuoteBody(report) => &report.report_data,
        };

        if quote_report_data != &calculated_report_data {
            warn!("TDX Quote report_data mismatch:");
            warn!("  - Expected hash (from payload): {}", hex::encode(&calculated_report_data[..32]));
            warn!("  - Got hash (from quote report_data): {}", hex::encode(&quote_report_data[..32]));
            return Err(VerificationError::TdxQuoteBindingMismatch);
        }
        info!("{}", "TDX Quote binding VERIFIED.".green());
    }
    // Call the DCAP verification function from runtime
    runtime::tee_attestation::perform_dcap_verification(&quote)
        .map_err(VerificationError::from)?;
    // --- End TDX Quote Verification ---

    // Return the deserialized payload data if all verifications succeed
    Ok(payload_data)
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
    fn create_mock_signed_report(signing_key: &SigningKey) -> (UsageReportPayload, SignedUsageReport) {
        // Create test usage data
        let usage = UsageData {
            reverie_id: Some(String::from("reverie_123")),
            spender: Some(String::from("spender_123")),
            spender_type: Some(String::from("escdsa")),
            input_tokens: 100,
            output_tokens: 250,
            cache_creation_input_tokens: Some(50),
            cache_read_input_tokens: None,
            tool_use: None,
        };

        // Create payload
        let payload = UsageReportPayload {
            usage,
            timestamp: 1745468461, // Fixed timestamp for consistent tests
            linked_tool_use_id: None,
            request_id: String::from("request_121234"),
        };

        // Serialize and sign
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let signature: Signature = signing_key.sign(&payload_bytes);

        let mock_quote_tuple = if cfg!(test) {
            // Generate mock quote with correct report_data
            runtime::tee_attestation::generate_tee_attestation_with_data([0; 64], false)
                .expect("Mock TEE attestation failed")
        } else {
            // This branch should not be hit during tests. Panic if it is.
            panic!("create_signed_report called outside of test context!");
            // (QuoteV4::default(), vec![0u8; 100]) // Remove default usage
        };
        // --- Use the bytes (.1) from the tuple for encoding ---
        let tdx_quote_b64 = base64_standard.encode(&mock_quote_tuple.1);
        // --- End Mock TDX Quote ---

        // Create signed report
        let signed_report = SignedUsageReport {
            payload: base64_standard.encode(&payload_bytes),
            signature: base64_standard.encode(signature.to_bytes()),
            tdx_quote: tdx_quote_b64, // Assign the base64 string
        };

        (payload, signed_report)
    }

    #[test]
    fn test_verification_success() {
        // Generate keypair
        let (signing_key, verifying_key) = generate_test_keypair();

        // Create and sign report
        let (
            expected_payload,
            signed_report
        ) = create_mock_signed_report(&signing_key);

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
        let (_, signed_report) = create_mock_signed_report(&signing_key);

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
        let (_, mut signed_report) = create_mock_signed_report(&signing_key);

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
        let (_, mut signed_report) = create_mock_signed_report(&signing_key);

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