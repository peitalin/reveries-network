export interface TD10QuoteBody {
  tee_tcb_svn: number[];
  mrseam: number[];
  mrsignerseam: number[];
  seam_attributes: number;
  td_attributes: number;
  xfam: number;
  mrtd: number[];
  mrconfigid: number[];
  mrowner: number[];
  mrownerconfig: number[];
  rtmr0: number[];
  rtmr1: number[];
  rtmr2: number[];
  rtmr3: number[];
  report_data: number[];
}

export interface CertData {
  cert_data_type: number;
  cert_data_size: number;
  cert_data: number[];
}

export interface QuoteSignatureDataV4 {
  quote_signature: string; // hex string
  ecdsa_attestation_key: string; // hex string
  qe_cert_data: CertData;
}

export interface TeeQuoteHeader {
  version: number;
  att_key_type: number;
  tee_type: number;
  qe_svn: number[];
  pce_svn: number[];
  qe_vendor_id: number[];
  user_data: number[];
}

export interface TeeQuoteV4Attestation {
  header: TeeQuoteHeader;
  quote_body: TD10QuoteBody;
  signature: QuoteSignatureDataV4;
  signature_len: string;
  time: {
    secs: number;
    nanos: number;
  };
}

export interface HeartbeatData {
  node_state: NodeState; // This is a dynamic JSON value from the server
  tee_attestation: {
    peer_id: string;
    peer_name: string;
    tee_quote_v4: TeeQuoteV4Attestation;
  };
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

export interface CFrag {
  alice_pk: string;
  bob_pk: string;
  cfrag: string;
  frag_num: number;
  next_vessel_peer_id: string;
  sender_peer_id: string;
  threshold: number;
  verifying_pk: string;
  vessel_peer_id: string;
}

export interface KFragBroadcastPeers {
  frag_num: number;
  peers: string[];
}

export interface PeerInfo {
  node_name: string;
  peer_id: string;
  agent_vessel: {
    agent_name_nonce: string;
    next_vessel: string;
    prev_vessel: string;
    total_frags: number;
  };
  heartbeat_data: {
    last_hb: {
      secs: number;
      nanos: number;
    };
    tee_byte_len: number;
  };
}

export interface NodeState {
  _node_name: string;
  _peer_id: string;
  _umbral_public_key: string;
  _agent_in_vessel: {
    agent_name_nonce: string;
    total_frags: number;
    current_vessel_node_name: string;
    current_vessel_peer_id: string;
    next_vessel_node_name: string;
    next_vessel_peer_id: string;
  };
  _pending_respawns: Array<{
    next_vessel_peer_id: string;
    prev_agent: string;
  }>;
  peer_manager: {
    '1_cfrags_summary'?: Array<{
      agent_name_nonce: string;
      cfrag: CFrag;
    }>;
    '2_kfrag_broadcast_peers'?: Array<{
      agent_name_nonce: string;
      kfrag_broadcast_peers: Array<KFragBroadcastPeers>;
    }>;
    '3_peer_info'?: PeerInfo[];
  };
}

export interface PeerManagerData {
  cfrags_summary: Array<{
    agent_name_nonce: string;
    cfrag: CFrag;
  }>;
  kfrag_broadcast_peers: Array<{
    agent_name_nonce: string;
    kfrag_broadcast_peers: Array<KFragBroadcastPeers>;
  }>;
  peer_info: PeerInfo[];
}

export interface WebSocketConnection {
  port: number;
  heartbeat: HeartbeatData | null;
  isConnected: boolean;
  error: string | null;
  isLoading?: boolean;
}

// Specific type for heartbeat subscription responses
export interface HeartbeatResponse extends JsonRpcResponse<HeartbeatData> {}