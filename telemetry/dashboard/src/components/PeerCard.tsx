import React from 'react';
import { PeerInfo, HeartbeatData } from '../types';
import { formatLastSeen } from '../utils/formatting';

interface PeerCardProps {
  peer: PeerInfo;
  heartbeat: HeartbeatData;
  isExpanded: boolean;
  onToggleExpand: () => void;
  getPeerManagerData: (data: HeartbeatData) => any;
}

export const PeerCard: React.FC<PeerCardProps> = ({
  peer,
  heartbeat,
  isExpanded,
  onToggleExpand,
  getPeerManagerData
}) => {
  return (
    <div
      className="p-4 bg-gray-800 rounded-lg cursor-pointer transition-all hover:bg-gray-700"
      onClick={onToggleExpand}
    >
      <div className="font-bold">{peer.node_name}</div>
      <div className="text-sm text-gray-400 mb-2">ID: {peer.peer_id}</div>
      <div className="text-sm">
        Last seen: {formatLastSeen(peer.heartbeat_data?.last_hb)}
      </div>
      {isExpanded && (
        <div className="mt-3 pt-3 border-t border-gray-600">
          <h4 className="text-sm font-semibold mb-2">Peer Info:</h4>
          <pre className="text-xs bg-gray-900 p-2 rounded overflow-auto">
            {JSON.stringify(peer, null, 2)}
          </pre>

          <h4 className="text-sm font-semibold mt-4 mb-2">KFrag Broadcast Peers:</h4>
          <pre className="text-xs bg-gray-900 p-2 rounded overflow-auto">
            {JSON.stringify(getPeerManagerData(heartbeat).kfrag_broadcast_peers)}
          </pre>

          <h4 className="text-sm font-semibold mt-4 mb-2">CFrag Summary:</h4>
          <pre className="text-xs bg-gray-900 p-2 rounded overflow-auto">
            {JSON.stringify(getPeerManagerData(heartbeat).cfrags_summary)}
          </pre>
        </div>
      )}
    </div>
  );
};