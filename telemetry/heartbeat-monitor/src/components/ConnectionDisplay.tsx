import React from 'react';
import { HeartbeatData, PeerManagerData } from '../types';
import { formatTime, getColorForPeerName } from '../utils/formatting';
import { PeerCard } from './PeerCard';

interface ConnectionDisplayProps {
  port: number;
  heartbeat: HeartbeatData | null;
  isConnected: boolean;
  error: string | null;
  expandedPeerId: string;
  setExpandedPeerId: (peerId: string) => void;
  getPeerManagerData: (data: HeartbeatData) => PeerManagerData;
  isLoading?: boolean;
}

const BackgroundOverlay: React.FC<{nodeColor: string | undefined}> = ({nodeColor}) => {
  if (!nodeColor) return null;
  return (
    <div className={`absolute w-full h-full top-0 left-0 opacity-40`}
      style={{backgroundColor: nodeColor}}
    />
  );
};

export const ConnectionDisplay: React.FC<ConnectionDisplayProps> = ({
  port,
  heartbeat,
  isConnected,
  error,
  expandedPeerId,
  setExpandedPeerId,
  getPeerManagerData,
  isLoading
}) => {
  const nodeColor = heartbeat ? getColorForPeerName(heartbeat.node_state._node_name) : undefined;

  return (
    <div className="p-2 rounded-md relative">
      <div className={`absolute w-full h-full top-0 left-0 opacity-40`} style={{backgroundColor: nodeColor}}></div>
      <BackgroundOverlay nodeColor={nodeColor}/>
      <div className="p-4 rounded-md mb-2 bg-gray-700 relative">
        <div className="flex items-center gap-4 mb-4">
          <h2 className={`inline-block px-3 py-2 rounded-md text-lg font-bold ${
            isLoading ? 'bg-yellow-600' : isConnected ? 'bg-green-600' : 'bg-red-600'
          }`}>
            {heartbeat?.node_state._node_name} {isLoading ? 'Connecting...' : isConnected ? 'Connected' : 'Disconnected'}
          </h2>
          <h3 className="text-xl font-bold p-2 rounded">
            Port {port}
            {isLoading && <span className="ml-2 text-yellow-500">(Connecting...)</span>}
            {isConnected && <span className="ml-2 text-green-500">(Connected)</span>}
            {!isConnected && !isLoading && <span className="ml-2 text-red-500">(Disconnected)</span>}
            {!isConnected && !isLoading && !!error && <span className="ml-2 text-red-500">(Error: {error})</span>}
          </h3>
        </div>

        {heartbeat && (
          <div className="text-sm mt-2">
            <div><span className="font-bold">PeerID:</span> {heartbeat.node_state._peer_id}</div>
            <div className="truncate"><span className="font-bold">Umbral Public Key:</span> {heartbeat.node_state._umbral_public_key}</div>
            <div className="mt-0">
              <div className="font-bold">Agent in Vessel:</div>
              <pre className="text-xs bg-black bg-opacity-50 p-2 rounded mt-1 mb-2 overflow-x-auto">
                {JSON.stringify(heartbeat.node_state._agent_in_vessel, null, 2)}
              </pre>
            </div>
            <div className="mt-0">
              <div className="font-bold">Pending Respawns:</div>
              <pre className="text-xs bg-black bg-opacity-50 p-2 rounded mt-1 overflow-x-auto">
                {JSON.stringify(heartbeat.node_state._pending_respawns, null, 2)}
              </pre>
            </div>
          </div>
        )}
      </div>

      {heartbeat && (
        <div className="relative">
          <div className="my-2 p-4 bg-gray-700 rounded-md">
            <h3 className="text-xl font-semibold mb-2">
              {heartbeat.node_state._node_name}'s Connected Peers ({getPeerManagerData(heartbeat).peer_info.length})
            </h3>
            <div className="grid gap-2 grid-cols-1 md:grid-cols-2 lg:grid-cols-3">
              {getPeerManagerData(heartbeat).peer_info.map((peer) => (
                <PeerCard
                  key={peer.peer_id}
                  peer={peer}
                  heartbeat={heartbeat}
                  isExpanded={expandedPeerId === '*' || expandedPeerId === peer.peer_id}
                  onToggleExpand={() => setExpandedPeerId(expandedPeerId === '*' ? peer.peer_id : expandedPeerId === peer.peer_id ? '' : '*')}
                  getPeerManagerData={getPeerManagerData}
                />
              ))}
            </div>
          </div>

          <div className="my-2 p-4 bg-gray-700 rounded-md">
            <h3 className="text-xl font-semibold mb-2">Latest TEE Attestation</h3>
            <pre className="text-sm overflow-auto bg-gray-900 p-2 rounded-md">{JSON.stringify(heartbeat.tee_attestation, null, 2)}</pre>
          </div>

          <div className="p-4 bg-gray-700 rounded-md">
            <h3 className="text-xl font-semibold mb-2">Timestamp</h3>
            <pre className="text-sm overflow-auto bg-gray-900 p-2 rounded-m">{formatTime(heartbeat.time)}</pre>
          </div>
        </div>
      )}
    </div>
  );
};