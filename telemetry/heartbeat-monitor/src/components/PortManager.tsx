import React from 'react';

interface PortManagerProps {
  ports: string[];
  newPort: string;
  teeUrl: string;
  onPortChange: (value: string) => void;
  onTeeUrlChange: (value: string) => void;
  onPortSubmit: (e: React.FormEvent) => void;
  onPortRemove: (port: number) => void;
}

export const PortManager: React.FC<PortManagerProps> = ({
  ports,
  newPort,
  teeUrl,
  onPortChange,
  onTeeUrlChange,
  onPortSubmit,
  onPortRemove
}) => {
  return (
    <div className="mb-2 p-4 bg-gray-700 rounded-lg">
      <h3 className="text-lg font-semibold mb-2">Manage Connections</h3>
      <form onSubmit={onPortSubmit} className="flex gap-2 mb-4">
        <input
          type="text"
          value={teeUrl}
          onChange={(e) => onTeeUrlChange(e.target.value)}
          placeholder="TEE URL (e.g., 0.0.0.0)"
          className="px-3 py-2 bg-gray-800 rounded text-white w-48"
        />
        <input
          type="number"
          value={newPort}
          onChange={(e) => onPortChange(e.target.value)}
          placeholder="Enter port number"
          className="px-3 py-2 bg-gray-800 rounded text-white w-48"
          min="1"
          max="65535"
        />
        <button
          type="submit"
          className="px-4 py-2 bg-blue-600 rounded hover:bg-blue-700 transition-colors"
        >
          Add Connection
        </button>
      </form>

      <div className="flex flex-wrap gap-2">
        {ports.map(port => (
          <div key={port} className="flex items-center gap-2 bg-gray-800 px-3 py-1 rounded">
            <span>ws://{teeUrl}:{port}</span>
            <button
              onClick={() => onPortRemove(Number(port))}
              className="ml-2 text-red-400 hover:text-red-300"
            >
              ×
            </button>
          </div>
        ))}
      </div>
    </div>
  );
};