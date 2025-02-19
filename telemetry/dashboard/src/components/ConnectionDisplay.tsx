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
      className="rounded-lg p-4"
      style={{
        backgroundColor: nodeColor,
        opacity: 0.9
      }}
    >
      <div className="p-3 rounded-md mb-3 bg-gray-700">
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
            <div className="grid gap-4 grid-cols-1 md:grid-cols-2 lg:grid-cols-3">
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

          <div className="my-3 p-3 bg-gray-700 rounded-md">
            <h3 className="text-xl font-semibold mb-2">Latest TEE Attestation</h3>
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