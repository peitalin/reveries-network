import React, { useEffect, useState } from 'react';
import { NodeState, HeartbeatData, WebSocketConnection, PeerManagerData } from '../types';
import { JsonRpcWebSocket } from '../utils/websocket';
import { formatTime, getColorForPeerName, formatHeartbeatData, getLastSeenDiff } from '../utils/formatting';
import toast, { Toaster } from 'react-hot-toast';
import { PortManager } from './PortManager';
import { ConnectionDisplay } from './ConnectionDisplay';

const HeartbeatMonitor: React.FC = () => {
  const [connections, setConnections] = useState<Record<number, WebSocketConnection>>({});
  const [expandedPeerId, setExpandedPeerId] = useState<string>('*');
  const [newPort, setNewPort] = useState<string>('');

  // Function to add a new port
  const addPort = async (port: number) => {
    if (connections[port]) {
      toast.error(`Port ${port} is already being monitored`);
      return;
    }
    // Set initial state first
    setConnections(prev => ({
      ...prev,
      [port]: {
        port,
        heartbeat: null,
        isConnected: false,
        error: null,
        isLoading: true
      }
    }));

    // Wait for state update to be applied
    await new Promise(resolve => setTimeout(resolve, 0));

    // Then attempt connection
    await connectWebSocket(port);
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

  const getPeerManagerData = (data: HeartbeatData): PeerManagerData => {
    if (!data) return { cfrags_summary: [], kfrag_broadcast_peers: [], peer_info: [] };
    const nodeState: NodeState = data.node_state || {};
    const peerManager: {
      '1_cfrags_summary'?: any[];
      '2_kfrag_broadcast_peers'?: any[];
      '3_peer_info'?: any[];
    } = nodeState.peer_manager || {};

    return {
      cfrags_summary: peerManager['1_cfrags_summary'] || [],
      kfrag_broadcast_peers: peerManager['2_kfrag_broadcast_peers'] || [],
      peer_info: peerManager['3_peer_info'] || []
    };
  };

  const getToastStyle = (peerName: string) => {
    return {
      background: getColorForPeerName(peerName),
      color: '#fff',
      borderRadius: '3px',
      padding: '6px 12px',
    };
  };

  const connectWebSocket = async (port: number) => {
    try {
      wsClients[port] = new JsonRpcWebSocket(`ws://0.0.0.0:${port}`);

      wsClients[port]?.onClose(() => {
        console.log(`WebSocket closed for port ${port}`);
        setConnections(prev => ({
          ...prev,
          [port]: {
            ...prev[port],
            heartbeat: null,
            isConnected: false,
            error: 'Connection closed',
            isLoading: true
          }
        }));

        // Attempt to reconnect
        setTimeout(() => {
          connectWebSocket(port);
        }, 1000);
      });

      await wsClients[port]?.subscribe(
        'subscribe_hb',
        [0],
        (raw_data: any) => {
          const data = formatHeartbeatData(raw_data);

          setConnections(prev => ({
            ...prev,
            [port]: {
              ...prev[port],
              heartbeat: data,
              isConnected: true,
              error: null,
              isLoading: false  // Connection successful, stop loading
            }
          }));

          const tee_attestation = data.tee_attestation;
          const lastSeenTime = tee_attestation.tee_quote_v4?.time;

          toast.success(
            <div>
              <div className="font-bold">{tee_attestation.peer_name}</div>
              <div className="text-sm">Time: {formatTime(data.time)}</div>
              <div className="text-sm">Last seen: {getLastSeenDiff(lastSeenTime)}</div>
              <div className="text-sm">
                Signature: <span className="break-all">{tee_attestation.tee_quote_v4?.signature?.quote_signature || 'N/A'}</span>
              </div>
            </div>,
            {
              duration: 3000,
              position: 'bottom-right',
              style: getToastStyle(tee_attestation.peer_name),
            }
          );
        }
      );
    } catch (err) {
      setConnections(prev => ({
        ...prev,
        [port]: {
          ...prev[port],
          error: err instanceof Error ? err.message : 'Failed to connect',
          isConnected: false,
          isLoading: false  // Connection failed, stop loading
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
    const initializeConnections = async () => {
      const params = new URLSearchParams(window.location.search);
      const portParam = params.get('port');
      const defaultPorts = portParam ?
        portParam.split(',').map(p => parseInt(p.trim())).filter(p => !isNaN(p)) :
        [8001, 8002];

      for (const port of defaultPorts) {
        if (port > 0 && port < 65536) {
          await addPort(port);
        }
      }
    };

    initializeConnections();

    return () => {
      Object.values(wsClients).forEach(client => {
        if (client) client.unsubscribe();
      });
    };
  }, []);

  return (
    <div className="bg-gray-800 text-white">
      <div className="container mx-auto">
        <Toaster
          position="top-right"
          toastOptions={{
            duration: 3000,
            style: {
              background: '#374151',
              color: '#fff',
            },
          }}
        />
      </div>

      {/* Port Management UI */}
      <div className="container p-4">
        <h2 className="text-2xl font-bold my-2 mx-1">Node Heartbeat Monitor</h2>
        <PortManager
          ports={Object.keys(connections)}
          newPort={newPort}
          onPortChange={setNewPort}
          onPortSubmit={handleSubmit}
          onPortRemove={removePort}
        />
      </div>

      {/* Existing connection displays */}
      {Object.values(connections).map(({ port, heartbeat, isConnected, error, isLoading }) => (
        <ConnectionDisplay
          key={port}
          port={port}
          heartbeat={heartbeat}
          isConnected={isConnected}
          isLoading={isLoading}
          error={error}
          expandedPeerId={expandedPeerId}
          setExpandedPeerId={setExpandedPeerId}
          getPeerManagerData={getPeerManagerData}
        />
      ))}
    </div>
  );
};

export default HeartbeatMonitor;