import React, { useEffect, useState } from 'react';
import { HeartbeatData } from '../types';
import { JsonRpcWebSocket } from '../utils/websocket';
import toast, { Toaster } from 'react-hot-toast';

interface NodeState {
  _node_name: string;
  _peer_id: string;
  _umbral_public_key: string;
  peer_manager: {
    '1_cfrags_summary'?: Array<{
      agent_name_nonce: string;
      cfrag: {
        alice_pk: string;
        bob_pk: string;
        cfrag: string;
        frag_num: number;
        next_vessel_peer_id: string;
        sender_peer_id: string;
        threshold: number;
        verifying_pk: string;
        vessel_peer_id: string;
      };
    }>;
    '2_kfrag_broadcast_peers'?: Array<{
      agent_name_nonce: string;
      kfrag_broadcast_peers: Array<{
        frag_num: number;
        peers: string[];
      }>;
    }>;
    '3_peer_info'?: Array<{
      agent_vessel: {
        agent_name: string;
        next_vessel: string;
        prev_vessel: string;
        total_frags: number;
      } | null;
      heartbeat_data: {
        last_hb: {
          nanos: number;
          secs: number;
        };
        tee_byte_len: number;
      };
      node_name: string;
      peer_id: string;
    }>;
  };
}

interface PeerInfo {
  node_name: string;
  peer_id: string;
  agent_vessel: {
    agent_name: string;
    next_vessel: string;
    prev_vessel: string;
    total_frags: number;
  } | null;
  heartbeat_data: {
    last_hb: {
      secs: number;
      nanos: number;
    };
    tee_byte_len: number;
  };
}

interface PeerManagerData {
  cfrags_summary: Array<{
    agent_name_nonce: string;
    cfrag: {
      alice_pk: string;
      bob_pk: string;
      cfrag: string;
      frag_num: number;
      next_vessel_peer_id: string;
      sender_peer_id: string;
      threshold: number;
      verifying_pk: string;
      vessel_peer_id: string;
    };
  }>;
  kfrag_broadcast_peers: Array<{
    agent_name_nonce: string;
    kfrag_broadcast_peers: Array<{
      frag_num: number;
      peers: string[];
    }>;
  }>;
  peer_info: PeerInfo[];
}

const HeartbeatMonitor: React.FC = () => {
  const [heartbeat, setHeartbeat] = useState<HeartbeatData | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expandedPeerId, setExpandedPeerId] = useState<string | null>(null);

  const formatTime = (time: any) => {
    if (time && typeof time === 'object' && 'secs' in time) {
      return new Date(time.secs * 1000).toLocaleString();
    }
    return JSON.stringify(time);
  };

  const formatLastSeen = (lastHb?: { secs: number; nanos: number }) => {
    if (!lastHb) return 'Never';

    const timeDiffSeconds = lastHb.secs; // already the time difference in seconds
    const milliseconds = Math.floor(lastHb.nanos / 1_000_000); // Convert nanoseconds to milliseconds

    if (timeDiffSeconds < 60) {
      // For times less than a minute, show seconds with millisecond precision
      return `${timeDiffSeconds}.${milliseconds.toString().padStart(3, '0')}s ago`;
    }

    const minutes = Math.floor(timeDiffSeconds / 60);
    const remainingSeconds = timeDiffSeconds % 60;
    // For times over a minute, show minutes and seconds with millisecond precision
    return `${minutes}m ${remainingSeconds}.${milliseconds.toString().padStart(3, '0')}s ago`;
  };

  const getPeerManagerData = (data: HeartbeatData): PeerManagerData => {
    if (!data) return { cfrags_summary: [], kfrag_broadcast_peers: [], peer_info: [] };
    const nodeState = data.node_state || {};
    const peerManager = nodeState.peer_manager || {};

    return {
      cfrags_summary: peerManager['1_cfrags_summary'] || [],
      kfrag_broadcast_peers: peerManager['2_kfrag_broadcast_peers'] || [],
      peer_info: peerManager['3_peer_info'] || []
    };
  };

  useEffect(() => {
    let wsClient: JsonRpcWebSocket | null = null;

    const connectWebSocket = async () => {
      try {
        wsClient = new JsonRpcWebSocket("ws://0.0.0.0:8002");
        await wsClient.subscribe(
          'subscribe_hb',
          [0],
          (data: HeartbeatData) => {
            setHeartbeat(data);
            setIsConnected(true);
            setError(null);

            const peers = getPeerManagerData(data).peer_info;

            // Show toast for each peer
            peers.forEach(peer => {
              toast.success(
                <div>
                  <div className="font-bold">{peer.node_name}</div>
                  <div className="text-sm text-gray-300">ID: {peer.peer_id}</div>
                  <div>Time: {formatTime(data.time)}</div>
                  <div>Last seen: {formatLastSeen(peer.heartbeat_data?.last_hb)}</div>
                </div>,
                {
                  duration: 3000,
                  position: 'bottom-right',
                  style: {
                    background: '#1f2937',
                    color: '#fff',
                    borderRadius: '8px',
                    padding: '12px 24px',
                  },
                }
              );
            });
          }
        );
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to connect');
        setIsConnected(false);
      }
    };

    connectWebSocket();

    return () => {
      if (wsClient) {
        wsClient.unsubscribe();
      }
    };
  }, []);

  const peers = heartbeat ? getPeerManagerData(heartbeat).peer_info : [];

  return (
    <div className="p-5 bg-gray-800 text-white rounded-lg m-5">
      {/* Add Toaster component */}
      <Toaster />

      <h2 className="text-2xl font-bold mb-4">Node Heartbeat Monitor</h2>

      <div className={`p-3 rounded-md mb-3 ${
        isConnected ? 'bg-green-600' : 'bg-red-600'
      }`}>
        Status: {isConnected ? 'Connected' : 'Disconnected'}
      </div>

      {error && (
        <div className="p-3 bg-red-600 rounded-md mb-3">
          Error: {error}
        </div>
      )}

      {heartbeat && (
        <>
          <div className="my-3 p-3 bg-gray-700 rounded-md">
            <h3 className="text-xl font-semibold mb-2">
              {heartbeat.node_state._node_name}'s Connected Peers ({peers.length})
            </h3>
            <div className="text-sm text-gray-400 mb-4">
              <div>PeerID: {heartbeat.node_state._peer_id}</div>
              <div className="truncate">Reencryption Public Key: {heartbeat.node_state._umbral_public_key}</div>
            </div>
            <div className="grid gap-4 grid-cols-1 md:grid-cols-2 lg:grid-cols-3">
              {peers.map((peer) => (
                <div
                  key={peer.peer_id}
                  className="p-4 bg-gray-800 rounded-lg cursor-pointer transition-all hover:bg-gray-700"
                  onClick={() => setExpandedPeerId(expandedPeerId === peer.peer_id ? null : peer.peer_id)}
                >
                  <div className="font-bold">{peer.node_name}</div>
                  <div className="text-sm text-gray-400 mb-2">ID: {peer.peer_id}</div>
                  <div className="text-sm">
                    Last seen: {formatLastSeen(peer.heartbeat_data?.last_hb)}
                  </div>

                  {expandedPeerId === peer.peer_id && (
                    <div className="mt-3 pt-3 border-t border-gray-600">
                      <h4 className="text-sm font-semibold mb-2">Full Peer Details:</h4>
                      <pre className="text-xs bg-gray-900 p-2 rounded overflow-auto">
                        {JSON.stringify(peer, null, 2)}
                      </pre>
                    </div>
                  )}
                </div>
              ))}
            </div>
          </div>

          <div className="my-3 p-3 bg-gray-700 rounded-md">
            <h3 className="text-xl font-semibold mb-2">TEE Attestation</h3>
            <pre className="overflow-auto">{JSON.stringify(heartbeat.tee_attestation, null, 2)}</pre>
          </div>

          <div className="my-3 p-3 bg-gray-700 rounded-md">
            <h3 className="text-xl font-semibold mb-2">Timestamp</h3>
            <pre className="overflow-auto">{formatTime(heartbeat.time)}</pre>
          </div>
        </>
      )}
    </div>
  );
};

export default HeartbeatMonitor;