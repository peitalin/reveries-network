
### MPC network + Heartbeat demo


### TEE VM Setup

TDX enabled confidential VMs can only be setup with `gcloud`, they can't be accessed by the UI.
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
gcloud compute instances create tee3-instance-20250123-042942 \
    --project=eigen-413918 \
    --confidential-compute-type=TDX \
    --zone=asia-southeast1-a \
    --machine-type=c3-standard-4 \
    --maintenance-policy=TERMINATE \
    --service-account=634774300751-compute@developer.gserviceaccount.com \
    --tags=http-server,https-server \
    --create-disk=auto-delete=yes,boot=yes,device-name=tee3-instance-20250123-042942,image=projects/ubuntu-os-cloud/global/images/ubuntu-2204-jammy-v20250112,mode=rw,provisioned-iops=3060,provisioned-throughput=155,size=10,type=hyperdisk-balanced \
    --no-shielded-secure-boot \
    --shielded-vtpm \
    --shielded-integrity-monitoring \
    --labels=goog-ec-src=vm_add-gcloud \
    --reservation-affinity=any
```




#### RPC Node Server

The following commands should be run in different terminals. You can also run JustFile commands.


terminal 1 - RPC server node 1
```
cargo run --bin rpc -- \
    --listen-address /ip4/0.0.0.0/tcp/40111 \
    --secret-key-seed 1 \
    --topics chat,kfrag0.bob,kfrag1.bob,kfrag2.bob,kfrag3.bob \
    --rpc-port 8001

```
terminal 2 - P2P node 2
```
cargo run --bin rpc -- \
    --listen-address /ip4/0.0.0.0/tcp/40222 \
    --secret-key-seed 2 \
    --topics chat,request.bob,broadcast.bob,kfrag0.bob \
    --rpc-port 8002

```
terminal 3 - P2P node 3
```
cargo run --bin rpc -- \
    --listen-address /ip4/0.0.0.0/tcp/40333 \
    --secret-key-seed 3 \
    --topics chat,request.bob,broadcast.bob,kfrag1.bob \
    --rpc-port 8003

```
terminal 4 - P2P node 4
```
cargo run --bin rpc -- \
    --listen-address /ip4/0.0.0.0/tcp/40444 \
    --secret-key-seed 4 \
    --topics chat,request.bob,broadcast.bob,kfrag2.bob \
    --rpc-port 8004

```

terminal 5 - P2P node 5
```
cargo run --bin rpc -- \
    --listen-address /ip4/0.0.0.0/tcp/40555 \
    --secret-key-seed 5 \
    --topics chat,request.bob,broadcast.bob,kfrag3.bob \
    --rpc-port 8005

```

NOTE: libp2p errors if you try to self-dial, so spin up 2 nodes and have them get the file from each other instead. If the RPC server is also the Node that hosts the file, it tries to self-dial and errors.



#### Cmd Client
Node1 and Node2 must be running
```
cargo run --bin cmd -- \
    --rpc-server-address 0.0.0.0:8001 \
    --agent-name bob \
    --shares 3 \
    --threshold 2
```


#### Runtime

Runtime that does the TEE attestations, houses the LLM, and generates Umbral keys

```
cargo run --bin runtime
```


#### Cait Sith Runtime

```
cargo run --release --bin cait -- 100 300 1000000
```