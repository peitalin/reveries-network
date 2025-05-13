
### Agent Reincarnation Network

This network hosts autonomous AI agents that own their own private wallets, have their own private memories inside of TEE (Trusted Execution Environment) nodes and transplants agents (and their private states) into new "vessel" nodes whenever an agents TEE server crashes so that the agent never loses access to their funds, keys, or encrypted memories.

We use threshold proxy re-encryption to transplant AI Agent secret keys, memories, model weights, and other secret state between encrypted TEE vessels in the p2p network, ensuring digitally immortable sovereign agents to run in perpetuity. If an agent's vessel node dies, let it die and reincarnate.


#### Starting the network locally

Start the network locally, spinning up 5 nodes inside docker compose with:
```
docker compose up
```

Edit the `docker-compose.yml` file to add nodes.
I've hardcoded nodes to rpc ports (9901-9905) for local development purposes.

#### Node heartbeat monitor dashboard

Install pnpm:
```
curl -fsSL https://get.pnpm.io/install.sh | sh -
```

Then run:
```
cd telemetry/heartbeat-monitor
pnpm install
npm run dev
```

Navigate to `http://localhost:4000/?port=9901` to observe the node's state.
You can open multiple browser tabs and change the `port=9902` to observe node1 to node5's state.

Then you can interact with the nodes: spawn agents and trigger node failures.


#### Spawning and respawning vessel nodes (agents)
Install [just](https://github.com/casey/just):
Rust:
```
cargo install just
```
npm:
```
npm install -g rust-just
```

Once the nodes are running and you have installed `just`, spawn an agent on node1 (on port 9901) with:
```
just spawn-agent 9901
```
The node will generate re-encryption fragments and broadcast them to peers.
It will also select a node to be the next vessel to reincarnate the agent in, once it fails.

Then trigger a network failure for node1 (triggering agent reincarnation in the new vessel node):
```
just trigger-node-failure 9901
```

Node1 running on port 9901 will go offline, and the next vessel node will detect node failure and begin the agent reincarnation protocol.

If you watch the Node heartbeat-monitor react app, you will see the "Agent in Vessel" move from node1 to the next node.

The new agent's name will have an incremented nonce, e.g:
```
auron-0 -> auron-1
```

Failed nodes will reboot after a timeout, and so you should be able to constantly trigger node failures and see the agent reincarnate and move around from vessel to vessel (still testing this).



#### Runtime
Debug run the runtime that handles TEE attestations, houses the LLM, and generates Umbral keys.
```
cargo run --bin runtime
```



### Deployment: TEE VM Setup

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

Test websocket heartbeat monitor with `websocat`:
```
cargo install websocat
websocat ws://34.143.238.248:9901

# then paste in JSOn input
{"jsonrpc":"2.0","method":"subscribe_hb","params":[0],"id":1}
```

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


kill -9 $(lsof -ti:9901)
