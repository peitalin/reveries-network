import React, { useEffect, useState, useMemo } from 'react';
import { NodeState, HeartbeatData, WebSocketConnection, PeerManagerData } from '../types';
import { JsonRpcWebSocket } from '../utils/websocket';
import { formatTime, getColorForPeerName, formatHeartbeatData, getLastSeenDiff } from '../utils/formatting';
import toast, { Toaster } from 'react-hot-toast';
import { PortManager } from './PortManager';
import { ConnectionDisplay } from './ConnectionDisplay';

const HeartbeatMonitor: React.FC = () => {
  // State
  const [connections, setConnections] = useState<Record<number, WebSocketConnection>>({});
  const [expandedPeerId, setExpandedPeerId] = useState<string>('*');
  const [teeUrl, setTeeUrl] = useState<string>(() => {
    const params = new URLSearchParams(window.location.search);
    return params.get('teeUrl') || '0.0.0.0';  // Get teeUrl from URL params or use default
  });
  const [newPort, setNewPort] = useState<string>('');
  const [lastHeartbeats, setLastHeartbeats] = useState<Record<number, number>>({});
  const wsClients = React.useRef<Record<number, JsonRpcWebSocket | null>>({}).current;

  // Config
  const heartbeatInterval = useMemo(() => {
    const params = new URLSearchParams(window.location.search);
    const intervalParam = params.get('interval');
    const interval = intervalParam ? parseInt(intervalParam) * 1000 : 10000;
    return Math.max(1000, interval);
  }, []);

  // Helper Functions
  const getToastStyle = (peerName: string) => ({
    background: getColorForPeerName(peerName),
    color: '#fff',
    borderRadius: '3px',
    padding: '4px 4px',
  });

  const getPeerManagerData = (data: HeartbeatData | null): PeerManagerData => {
    if (!data) return { cfrags_summary: [], kfrag_providers: [], peer_info: [] };
    const nodeState: NodeState = data.node_state || {};
    const peerManager = nodeState.peer_manager || {};
    return {
      cfrags_summary: peerManager['1_cfrags_summary'] || [],
      kfrag_providers: peerManager['2_kfrag_providers'] || [],
      peer_info: peerManager['3_peer_info'] || []
    };
  };

  // WebSocket Functions
  const connectWebSocket = async (port: number) => {
    try {
      let ws_url = `ws://${teeUrl}:${port}`;
      console.log(`Connecting to ${ws_url}`);
      wsClients[port] = new JsonRpcWebSocket(ws_url);

      wsClients[port]?.onOpen(() => {
        setLastHeartbeats(prev => ({ ...prev, [port]: Date.now() }));
      });

      wsClients[port]?.onClose(() => {
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
      });

      await wsClients[port]?.subscribe('subscribe_hb', [0], (raw_data: any) => {
        setLastHeartbeats(prev => ({ ...prev, [port]: Date.now() }));
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
            <div className="text-[12px] font-bold">{tee_attestation.peer_name}</div>
            <div className="text-[9px]">Time: {formatTime(data.time)}</div>
            <div className="text-[9px]">Last seen: {getLastSeenDiff(lastSeenTime)}</div>
            <div className="text-[9px]">
              Signature: <span className="break-all">{tee_attestation.tee_quote_v4?.signature?.quote_signature || 'N/A'}</span>
            </div>
          </div>,
          {
            duration: 3000,
            position: 'bottom-right',
            style: getToastStyle(tee_attestation.peer_name),
            icon: null,
          }
        );
      });
    } catch (err) {
      setConnections(prev => ({
        ...prev,
        [port]: {
          ...prev[port],
          error: err instanceof Error ? err.message : 'Failed to connect',
          isConnected: false,
          isLoading: false
        }
      }));
    }
  };

  // Port Management Functions
  const addPort = async (port: number) => {
    if (connections[port]) {
      toast.error(`Port ${port} is already being monitored`);
      return;
    }
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
    await new Promise(resolve => setTimeout(resolve, 0));
    await connectWebSocket(port);
  };

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

  // Effects
  useEffect(() => {
    const initializeConnections = async () => {
      const params = new URLSearchParams(window.location.search);
      const portParam = params.get('port');
      const defaultPorts = portParam ?
        portParam.split(',').map(p => parseInt(p.trim())).filter(p => !isNaN(p)) :
        [9901, 9902];

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

  useEffect(() => {
    const checkHeartbeats = () => {
      const now = Date.now();
      Object.entries(lastHeartbeats).forEach(([port, lastBeat]) => {
        const portNum = parseInt(port);
        if (now - lastBeat > heartbeatInterval) {
          toast(`No heartbeat for ${port} in ${heartbeatInterval/1000}s, reconnecting...`, {
            style: { background: '#333', color: '#fff', padding: '4px' }
          });
          if (wsClients[portNum]) {
            wsClients[portNum]?.unsubscribe();
            delete wsClients[portNum];
          }
          connectWebSocket(portNum);
        }
      });
    };

    const intervalId = setInterval(checkHeartbeats, Math.min(2000, heartbeatInterval/2));
    return () => clearInterval(intervalId);
  }, [lastHeartbeats, heartbeatInterval]);

  useEffect(() => {
    // Reconnect all existing connections when teeUrl changes
    Object.entries(connections).forEach(([port]) => {
      const portNum = parseInt(port);
      if (wsClients[portNum]) {
        wsClients[portNum]?.unsubscribe();
        delete wsClients[portNum];
      }
      connectWebSocket(portNum);
    });
  }, [teeUrl]); // Only run when teeUrl changes

  // Render
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
          teeUrl={teeUrl}
          onPortChange={setNewPort}
          onTeeUrlChange={setTeeUrl}
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