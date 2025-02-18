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

interface WebSocketConnection {
  port: number;
  heartbeat: HeartbeatData | null;
  isConnected: boolean;
  error: string | null;
}

const colors = [
  '#B22222', // FireBrick
  '#2E8B57', // SeaGreen
  '#4682B4', // SteelBlue
  '#8B008B', // DarkMagenta
  '#20B2AA', // LightSeaGreen
  '#CD853F', // Peru
  '#6A5ACD', // SlateBlue
  '#556B2F', // DarkOliveGreen
  '#B8860B', // DarkGoldenrod
  '#483D8B', // DarkSlateBlue
  '#8B4513', // SaddleBrown
  '#008B8B', // DarkCyan
  '#9932CC', // DarkOrchid
  '#3CB371', // MediumSeaGreen
  '#4B0082', // Indigo
  '#8B0000', // DarkRed
  '#2F4F4F', // DarkSlateGray
  '#7B68EE', // MediumSlateBlue
  '#A0522D', // Sienna
  '#5F9EA0', // CadetBlue
  '#6B8E23', // OliveDrab
  '#BC8F8F', // RosyBrown
  '#DAA520', // GoldenRod
  '#7F0000', // Maroon
  '#708090', // SlateGray
  '#4169E1', // RoyalBlue
  '#8FBC8F', // DarkSeaGreen
  '#D2691E', // Chocolate
  '#9370DB', // MediumPurple
  '#228B22'  // ForestGreen
];

const getColorForPeerId = (peerId: string) => {
  const shortId = peerId.slice(-8);

  const hash = shortId.split('').reduce((acc, char, i) => {
    return acc + char.charCodeAt(0) * (i + 1) * 31;
  }, 0);

  const color = colors[Math.abs(hash) % colors.length];
  console.log(`PeerId: ${peerId} -> Hash: ${hash} -> Color: ${color}`);
  return color;
};

const getToastPosition = (peerId: string): 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right' => {
  const hash = peerId.split('').reduce((acc, char) => char.charCodeAt(0) + acc, 0);
  const positions = ['top-left', 'top-right', 'bottom-left', 'bottom-right'];
  return positions[hash % positions.length] as 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right';
};

const HeartbeatMonitor: React.FC = () => {
  const [connections, setConnections] = useState<Record<number, WebSocketConnection>>({});
  const [expandedPeerId, setExpandedPeerId] = useState<string>('*');
  const [newPort, setNewPort] = useState<string>('');

  // Function to add a new port
  const addPort = (port: number) => {
    if (connections[port]) {
      toast.error(`Port ${port} is already being monitored`);
      return;
    }
    setConnections(prev => ({
      ...prev,
      [port]: { port, heartbeat: null, isConnected: false, error: null }
    }));
    connectWebSocket(port);
  };

  // Function to remove a port
  const removePort = (port: number) => {
    const client = wsClients[port];
    if (client) {
      client.unsubscribe();
      delete wsClients[port];
    }
    setConnections(prev => {
      const newConnections = { ...prev };
      delete newConnections[port];
      return newConnections;
    });
  };

  // Store WebSocket clients in a ref to maintain them across renders
  const wsClients = React.useRef<Record<number, JsonRpcWebSocket | null>>({}).current;

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

  const getToastStyle = (peerId: string) => {
    return {
      background: getColorForPeerId(peerId),
      color: '#fff',
      borderRadius: '8px',
      padding: '12px 24px',
    };
  };

  const connectWebSocket = async (port: number) => {
    try {
      wsClients[port] = new JsonRpcWebSocket(`ws://0.0.0.0:${port}`);
      await wsClients[port]?.subscribe(
        'subscribe_hb',
        [0],
        (data: HeartbeatData) => {
          setConnections(prev => ({
            ...prev,
            [port]: {
              ...prev[port],
              heartbeat: data,
              isConnected: true,
              error: null
            }
          }));

          const peers = getPeerManagerData(data).peer_info;
          peers.forEach(peer => {
            toast.success(
              <div>
                <div className="font-bold">{peer.node_name}</div>
                <div>Time: {formatTime(data.time)}</div>
                <div>Last seen: {formatLastSeen(peer.heartbeat_data?.last_hb)}</div>
              </div>,
              {
                duration: 3000,
                position: getToastPosition(data.node_state._peer_id),
                style: getToastStyle(data.node_state._peer_id),
              }
            );
          });
        }
      );
    } catch (err) {
      setConnections(prev => ({
        ...prev,
        [port]: {
          ...prev[port],
          error: err instanceof Error ? err.message : 'Failed to connect',
          isConnected: false
        }
      }));
    }
  };

  // Handle form submission
  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const port = parseInt(newPort);
    if (isNaN(port) || port < 1 || port > 65535) {
      toast.error('Please enter a valid port number (1-65535)');
      return;
    }
    addPort(port);
    setNewPort('');
  };

  useEffect(() => {
    // Get ports from URL query string
    const params = new URLSearchParams(window.location.search);
    const portParam = params.get('port');
    const defaultPorts = portParam ?
      portParam.split(',').map(p => parseInt(p.trim())).filter(p => !isNaN(p)) :
      [8001, 8002];  // fallback to default ports if none specified

    // Initialize with ports from URL or defaults
    defaultPorts.forEach(port => {
      if (port > 0 && port < 65536) {  // validate port range
        addPort(port);
      } else {
        toast.error(`Invalid port number: ${port}`);
      }
    });

    return () => {
      // Cleanup all connections
      Object.values(wsClients).forEach(client => {
        if (client) client.unsubscribe();
      });
    };
  }, []);  // Empty dependency array since we only want this to run once

  return (
    <div className="p-5 bg-gray-800 text-white">
      <Toaster />
      <h2 className="text-2xl font-bold mb-4">Node Heartbeat Monitor</h2>

      {/* Port Management UI */}
      <div className="mb-6 p-4 bg-gray-700 rounded-lg">
        <h3 className="text-lg font-semibold mb-3">Manage Ports</h3>
        <form onSubmit={handleSubmit} className="flex gap-2 mb-4">
          <input
            type="number"
            value={newPort}
            onChange={(e) => setNewPort(e.target.value)}
            placeholder="Enter port number"
            className="px-3 py-2 bg-gray-800 rounded text-white w-48"
            min="1"
            max="65535"
          />
          <button
            type="submit"
            className="px-4 py-2 bg-blue-600 rounded hover:bg-blue-700 transition-colors"
          >
            Add Port
          </button>
        </form>

        <div className="flex flex-wrap gap-2">
          {Object.keys(connections).map(port => (
            <div key={port} className="flex items-center gap-2 bg-gray-800 px-3 py-1 rounded">
              <span>Port {port}</span>
              <button
                onClick={() => removePort(Number(port))}
                className="ml-2 text-red-400 hover:text-red-300"
              >
                ×
              </button>
            </div>
          ))}
        </div>
      </div>

      {/* Existing connection displays */}
      {Object.values(connections).map(({ port, heartbeat, isConnected, error }) => {
        const nodeColor = heartbeat ? getColorForPeerId(heartbeat.node_state._peer_id) : undefined;
        console.log(`Node ${heartbeat?.node_state._node_name} color: ${nodeColor}`); // Debug log

        return (
          <div
            key={port}
            className="mb-8 rounded-lg p-4"
            style={{
              backgroundColor: nodeColor,
              opacity: 0.9
            }}
          >
            <div className={`p-3 rounded-md mb-3 ${
              isConnected ? 'bg-green-600' : 'bg-red-600'
            }`}>
              Node {heartbeat?.node_state._node_name} Status: {isConnected ? 'Connected' : 'Disconnected'}
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
                    {heartbeat.node_state._node_name}'s Connected Peers ({getPeerManagerData(heartbeat).peer_info.length})
                  </h3>
                  <div className="text-sm text-gray-400 mb-4">
                    <div>PeerID: {heartbeat.node_state._peer_id}</div>
                    <div className="truncate">Reencryption Public Key: {heartbeat.node_state._umbral_public_key}</div>
                  </div>
                  <div className="grid gap-4 grid-cols-1 md:grid-cols-2 lg:grid-cols-3">
                    {getPeerManagerData(heartbeat).peer_info.map((peer) => (
                      <div
                        key={peer.peer_id}
                        className="p-4 bg-gray-800 rounded-lg cursor-pointer transition-all hover:bg-gray-700"
                        onClick={() => setExpandedPeerId(expandedPeerId === '*' ? peer.peer_id : expandedPeerId === peer.peer_id ? '' : '*')}
                      >
                        <div className="font-bold">{peer.node_name}</div>
                        <div className="text-sm text-gray-400 mb-2">ID: {peer.peer_id}</div>
                        <div className="text-sm">
                          Last seen: {formatLastSeen(peer.heartbeat_data?.last_hb)}
                        </div>
                        {(expandedPeerId === '*' || expandedPeerId === peer.peer_id) && (
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
      })}
    </div>
  );
};

export default HeartbeatMonitor;