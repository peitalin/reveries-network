#!/bin/sh
# Use set -e to exit immediately if a command exits with a non-zero status.
# Use set -x to print each command before executing it (for debugging).
set -ex

# Check for node_key.json as a better indicator of successful init
NODE_KEY_FILE="/root/.near/node_key.json"
CONFIG_FILE="/root/.near/config.json" # Also check config file path
HOME_DIR="/root/.near"

# Replace with your actual public IP address
# IMPORTANT: THIS IS A PLACEHOLDER. YOU MUST SET YOUR ACTUAL PUBLIC IP.
# If this script is committed, consider fetching it from an env var or secure store.
YOUR_ACTUAL_PUBLIC_IP="YOUR_PUBLIC_IP_ADDRESS_HERE"

# Testnet boot nodes (ensure this list is up-to-date for 'testnet')
# From: https://docs.near.org/develop/node/config#boot-nodes
# Note: The jq solution is cleaner, but this sed fallback is for when jq isn't available.
TESTNET_BOOT_NODES="ed25519:86EtEy7epneKyrcJwSWP7zsisTkfDRH5CFVszt4qiQYw@35.195.32.249:24567,ed25519:BFB78VTDBBfCY4jCP99zWxhXUcFAZqR22oSx2KEr8UM1@35.229.222.235:24567,ed25519:Cw1YyiX9cybvz3yZcbYdG7oDV6D7Eihdfc8eM1e1KKoh@35.195.27.104:24567,ed25519:33g3PZRdDvzdRpRpFRZLyscJdbMxUA3j3Rf2ktSYwwF8@34.94.132.112:24567,ed25519:CDQFcD9bHUWdc31rDfRi4ZrJczxg8derCzybcac142tK@35.196.209.192:24567"

# Ensure the home directory exists (init might create it)
mkdir -p "$HOME_DIR"

if [ ! -f "$NODE_KEY_FILE" ]; then
  echo "Node key not found at $NODE_KEY_FILE, running initialization..."
  # Run init for testnet, download genesis.
  # If this command gets killed by OOM, the node_key file might not be created,
  # and this block will run again next time.
  neard --home "$HOME_DIR" init --chain-id testnet --download-genesis

  INIT_EXIT_CODE=$?
  if [ $INIT_EXIT_CODE -ne 0 ]; then
    echo "ERROR: 'neard init' command failed or was interrupted (exit code $INIT_EXIT_CODE)." >&2
    # Optionally clean up partial files if init failed partway?
    # rm -f "$HOME_DIR/config.json" "$HOME_DIR/genesis.json"
    exit $INIT_EXIT_CODE
  elif [ ! -f "$NODE_KEY_FILE" ]; then
     # Check again specifically for the key file after init supposedly succeeded
     echo "ERROR: 'neard init' completed but $NODE_KEY_FILE was not created. Init likely failed silently or was killed." >&2
     exit 1 # Exit with error
  fi
  echo "Initialization appears complete (node key found)."
else
  echo "Node key found at $NODE_KEY_FILE, skipping initialization."
fi

if [ ! -f "$CONFIG_FILE" ]; then
    echo "ERROR: $CONFIG_FILE still not found after init. This should not happen if init succeeded." >&2
    exit 1
fi

# Check and potentially inject boot_nodes if they are empty
current_boot_nodes_line=$(grep 'boot_nodes' "$CONFIG_FILE" | head -n 1 || echo "")
is_boot_nodes_empty=false

# Simplified and more robust grep check for empty boot_nodes
if echo "$current_boot_nodes_line" | grep -qE '"boot_nodes":\s*""' || \
   echo "$current_boot_nodes_line" | grep -qE '"boot_nodes":\s*\[[ ]*\]'; then
    is_boot_nodes_empty=true
fi

if [ "$is_boot_nodes_empty" = true ] ; then
  echo "Boot nodes are empty in $CONFIG_FILE. Attempting to inject testnet boot nodes..."
  if command -v jq > /dev/null; then
    # Create a temporary config, update it, then replace the original
    jq ".network.boot_nodes = \"$TESTNET_BOOT_NODES\"" "$CONFIG_FILE" > "$CONFIG_FILE.tmp" && mv "$CONFIG_FILE.tmp" "$CONFIG_FILE"
    echo "Injected testnet boot_nodes into $CONFIG_FILE using jq."
  else
    echo "WARNING: jq is not installed. Attempting to inject boot_nodes using sed (less reliable)." >&2
    sed -i "s|\"boot_nodes\": \"\"|\"boot_nodes\": \"$TESTNET_BOOT_NODES\"|" "$CONFIG_FILE"
    echo "Attempted to inject boot_nodes using sed."
  fi
else
  echo "Boot nodes appear to be populated in $CONFIG_FILE: $current_boot_nodes_line"
fi

# Check and set public_addrs if YOUR_ACTUAL_PUBLIC_IP is set and not a placeholder
if [ -n "$YOUR_ACTUAL_PUBLIC_IP" ] && [ "$YOUR_ACTUAL_PUBLIC_IP" != "YOUR_PUBLIC_IP_ADDRESS_HERE" ]; then
    echo "Setting public_addrs in $CONFIG_FILE..."
    if command -v jq > /dev/null; then
        public_addr_string="/ip4/$YOUR_ACTUAL_PUBLIC_IP/tcp/24567"
        # Ensure the .network object exists, then add/update public_addrs
        jq '(.network.public_addrs = []) | (.network.public_addrs += ["'"$public_addr_string"'"])' "$CONFIG_FILE" > "$CONFIG_FILE.tmp" && mv "$CONFIG_FILE.tmp" "$CONFIG_FILE"
        echo "Set network.public_addrs to [\"$public_addr_string\"] using jq."
    else
        echo "WARNING: jq is not installed. Cannot set public_addrs. Peering may fail if behind NAT." >&2
        # sed for adding/modifying public_addrs is very complex and error-prone; skipping sed fallback for this.
    fi
else
    echo "YOUR_ACTUAL_PUBLIC_IP not set or is placeholder; skipping public_addrs modification."
    echo "If behind NAT and peering fails, manually edit $CONFIG_FILE to set 'network.public_addrs = [\"/ip4/YOUR.PUBLIC.IP/tcp/24567\"]'"
fi

echo "Starting NEAR node..."
exec neard --home "$HOME_DIR" run