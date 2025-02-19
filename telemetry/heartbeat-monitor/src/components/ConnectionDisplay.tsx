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
}

export const ConnectionDisplay: React.FC<ConnectionDisplayProps> = ({
  port,
  heartbeat,
  isConnected,
  error,
  expandedPeerId,
  setExpandedPeerId,
  getPeerManagerData
}) => {
  const nodeColor = heartbeat ? getColorForPeerName(heartbeat.node_state._node_name) : undefined;

  return (
    <div
      key={port}
      className="p-2"
      style={{
        backgroundColor: nodeColor,
        opacity: 0.95
      }}
    >
      <div className="p-4 rounded-md mb-2 bg-gray-700">
        <div className={`inline-block px-2 py-1 rounded-md mb-2 text-lg font-medium ${
          isConnected ? 'bg-green-600' : 'bg-red-600'
        }`}>
          {heartbeat?.node_state._node_name} status: {isConnected ? 'Connected' : 'Disconnected'}
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

      {error && (
        <div className="p-4 bg-red-600 rounded-md mb-3">
          Error: {error}
        </div>
      )}

      {heartbeat && (
        <>
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

          <div className="my-2 p-4 bg-gray-700 rounded-md">
            <h3 className="text-xl font-semibold mb-2">Timestamp</h3>
            <pre className="text-sm overflow-auto bg-gray-900 p-2 rounded-m">{formatTime(heartbeat.time)}</pre>
          </div>
        </>
      )}
    </div>
  );
};