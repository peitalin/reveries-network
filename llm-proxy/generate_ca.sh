#!/bin/bash
# generate_ca.sh - Generates self-signed CA certificate and key.

set -e # Exit immediately if a command exits with a non-zero status.

# Determine the directory where the script resides
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

# Output directory relative to the script's location
CERT_DIR="${SCRIPT_DIR}/certs"
KEY_FILE="${CERT_DIR}/hudsucker.key"
CERT_FILE="${CERT_DIR}/hudsucker.cer"

# Ensure the certificate directory exists
mkdir -p "${CERT_DIR}"

# Generate the private key and self-signed certificate
# Overwrites existing files if they exist
openssl req -x509 -newkey rsa:4096 \
  -keyout "${KEY_FILE}" \
  -out "${CERT_FILE}" \
  -days 3650 \
  -nodes \
  -subj "/CN=LLM Proxy Generated CA"

echo "Successfully generated CA key and certificate in: ${CERT_DIR}"
echo "  Key: ${KEY_FILE}"
echo "  Cert: ${CERT_FILE}"

# Optional: Display cert details
# openssl x509 -in "${CERT_FILE}" -text -noout