import React from 'react';
import ReactDOM from 'react-dom/client';
import './index.css';
import HeartbeatMonitor from './components/HeartbeatMonitor';

const root = ReactDOM.createRoot(
  document.getElementById('root') as HTMLElement
);

root.render(
  <React.StrictMode>
    <div className="min-h-screen bg-gray-900">
      <header className="p-4 bg-gray-600">
        <h1 className="text-2xl font-bold text-white text-center">
          TEE Network Peer Tracker
        </h1>
      </header>
      <main className="w-full">
        <HeartbeatMonitor />
      </main>
    </div>
  </React.StrictMode>
);