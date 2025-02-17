export interface TeeAttestation {
  header: string;
  signature_len: string;
  time: number;
}

export interface HeartbeatData {
  node_state: any; // This is a dynamic JSON value from the server
  tee_attestation: TeeAttestation;
  time: number;
}

// Response format from the JSON-RPC server
export interface JsonRpcResponse<T> {
  jsonrpc: '2.0';
  method: string;
  params: {
    subscription: string;
    result: T;
  };
}

// Specific type for heartbeat subscription responses
export interface HeartbeatResponse extends JsonRpcResponse<HeartbeatData> {}