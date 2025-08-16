
### API Key Delegation Network

This network uses threshold proxy re-encryption (Umbral) to transplant secret keys, context, API Keys, and other secret state between TEE (Trusted Execution Environment) nodes in a p2p network.

In the first example, agents have private context and keys inside of TEE nodes and the network transplants their private states into new TEE "vessels" whenever their housing vessel/server crashes, to preserve access to private keys, or encrypted contexts.

We use the same memory delegation process to delegate encrypted API keys to TEE nodes running a `llm-proxy` that intercepts LLM API requests, injects API keys into requests, tracks token usage (to enable the use of onchain payments to pay for LLM API access).
- The hope is that we can tokenize LLM API_KEYs for lending and delegating LLM API access via onchain payments


#### Starting the network locally

Install [just](https://github.com/casey/just) with `cargo install just`.

Start the network locally, spinning up 5 nodes with either:

```
# terminal tab 1
just node1

# terminal tab 2
just node2

# ...

# terminal tab 5
just node5
```

Or with docker:
```
# Build p2p-node image
sh build-docker.sh

# Then run all 6 nodes:
docker-compose -f ./docker-compose-6-nodes.yaml up

# or use:
just run-local-nodes

```

#### Node heartbeat monitor dashboard

After launching the nodes, install [pnpm](https://pnpm.io/), then run:
```
just run-react-app
```

Navigate to `http://localhost:4000/?port=9901` to observe the node's state.
You can open multiple browser tabs and change the `port=9902` to observe node1 to node5's state.

Then you can interact with the nodes:

1) Spawn agent (with encrypted API key and private keys) on node1 on port 9901:
```
# The node generates re-encryption fragments and broadcasts them to peers.
just spawn-agent 9901
```

2) trigger node failure on node1 on port 9901:
```
just trigger-node-failure 9901
```

Node1 running on port 9901 will go offline, then the TEE heartbeats will timeout between the nodes and trigger the respawn process (threshold proxy re-encryption).

One of the peer nodes (a random node e.g `http://localhost:4000/?port=9902`) will now have the agent's decrypted secrets, you will see the "Agent in Vessel" move from node1 to the next node. The new agent's name will have an incremented nonce, e.g:
```
auron-0 -> auron-1
```

Failed nodes will reboot after a timeout, and so you should be able to constantly trigger node failures and see the agent reincarnate and move around from vessel to vessel (still testing this).

NOTE: the TEE attestations in the heartbeats will only be valid in TEE enabled VMs. If running locally, mock TEE attestations will be used instead.

You can also just run the e2e test for respawning an agent:
```
# Build p2p-node image
sh build-docker.sh

# run agent respawn test
just test-agent-respawn-after-failure
```


### Testing API Key Delegation (experimental)

If docker is installed, there are tests in `tests-e2e` module that outline the flow of:

1. Encrypt Anthropic API key, re-encrypt it, and broadcast to peers
2. Delegate Anthropic API key to a TEE node
3. Delegate secret context/memories for an agent
4. Execute LLM calls with secret memory and delegated API key
5. Measure token usage via llm-proxy and report back to the p2p-node

To run the test, move `env.example` to `.env` and add an Anthropic API KEY to `.env`
Then run:
```
# build llm-proxy and python MCP services
docker compose -f docker-compose-llm-proxy-test.yml build

# run test
just test-api-key-delegation
```

Future steps:
- integrate contracts for minting/tokenizing API keys
- onchain usage metrics, accounts, and billing for accessing delegated keys


## Deployment: TEE VM Setup

Deploy TDK confidential VMs on Google Cloud with terraform:
```
cd ./terraform

terraform init -upgrade
terraform plan
```
NOTE: you need the google-beta provider to deploy confidential VMs: https://registry.terraform.io/providers/hashicorp/google-beta/latest/docs/guides/provider_versions

The google-beta provider is added in the terraform/main.tf file.

Then once we are happy, execute:
```
terraform apply
```

The startup script in terraform should execute and install the required dependencies.
Monitor the node's executing deploy scripts after the VM is launched:
```
sh watch_nodes.sh 1
sh watch_nodes.sh 2
sh watch_nodes.sh 3
```

Once deployed, you can interact with the node via RPC:
```
curl -X POST http://<TEE_IP_ADDRESS>:9901 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"get_node_state","id":1}'
```

I will later put this RPC interface behind a proper HTTP API (caddy) on port 80.


### Misc Deployment Notes

TDX enabled confidential VMs can only be set up with `gcloud`, it cannot be accessed through the Google Cloud website.
[Setup google confidential VM settings via gcloud here](https://cloud.google.com/confidential-computing/confidential-vm/docs/create-a-confidential-vm-instance#gcloud)

```
gcloud compute instances create <VM-name-unique-id> \
    --confidential-compute-type=CONFIDENTIAL_COMPUTING_TECHNOLOGY \
    --machine-type=MACHINE_TYPE_NAME \
    --min-cpu-platform="CPU_PLATFORM" \
    --maintenance-policy="MAINTENANCE_POLICY" \
    --zone=ZONE_NAME \
    --image-family=IMAGE_FAMILY_NAME \
    --image-project=IMAGE_PROJECT \
    --project=PROJECT_ID
```

E.g
```
gcloud compute instances create tee09-instance-20250221-042942 \
    --project=eigen-413918 \
    --confidential-compute-type=TDX \
    --zone=asia-southeast1-a \
    --machine-type=c3-standard-4 \
    --maintenance-policy=TERMINATE \
    --service-account=634774300751-compute@developer.gserviceaccount.com \
    --tags=http-server,https-server \
    --create-disk=auto-delete=yes,boot=yes,device-name=tee09-instance-20250221-042942,image=projects/ubuntu-os-cloud/global/images/ubuntu-2204-jammy-v20250112,mode=rw,size=20,type=hyperdisk-balanced \
    --no-shielded-secure-boot \
    --shielded-vtpm \
    --shielded-integrity-monitoring \
    --labels=goog-ec-src=vm_add-gcloud \
    --reservation-affinity=any
```

