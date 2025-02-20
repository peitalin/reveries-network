import { HeartbeatData } from "../types";

// Color constants
export const colors = [
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

export const getColorForPeerName = (peerName: string): string => {
  // Use the full peer name for hashing since names are shorter than IDs
  const hash = peerName.split('').reduce((acc, char, i) => {
    return acc + char.charCodeAt(0) * (i + 1) * 31;
  }, 0);

  const color = colors[Math.abs(hash) % colors.length];
  return color;
};

export const getToastPosition = (peerId: string): 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right' => {
  const hash = peerId.split('').reduce((acc, char) => char.charCodeAt(0) + acc, 0);
  const positions = ['top-left', 'top-right', 'bottom-left', 'bottom-right'];
  return positions[hash % positions.length] as 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right';
};

// Existing time formatting functions
export const formatTime = (time: any) => {
  if (time && typeof time === 'object' && 'secs' in time) {
    return new Date(time.secs * 1000).toLocaleString();
  }
  return JSON.stringify(time);
};

export const formatLastSeen = (lastHb?: { secs: number; nanos: number }) => {
  if (!lastHb) return 'Never';

  const timeDiffSeconds = lastHb.secs;
  const milliseconds = Math.floor(lastHb.nanos / 1_000_000);

  if (timeDiffSeconds < 60) {
    return `${timeDiffSeconds}.${milliseconds.toString().padStart(3, '0')}s ago`;
  }

  const minutes = Math.floor(timeDiffSeconds / 60);
  const remainingSeconds = timeDiffSeconds % 60;
  return `${minutes}m ${remainingSeconds}.${milliseconds.toString().padStart(3, '0')}s ago`;
};

export function parseStructString(str: string) {
  // 1. Remove prefix word before the first {
  const objectStr = str.replace(/^\w+\s*/, '').trim();

  // 2. Clean up whitespace and normalize arrays
  const cleanStr = objectStr
    .replace(/\s+/g, ' ')
    .replace(/\[\s+/g, '[')
    .replace(/\s+\]/g, ']')
    .replace(/,\s+/g, ',');

  // 3. Convert to valid JSON by adding quotes around keys
  const jsonStr = cleanStr
    .replace(/([{,]\s*)([a-zA-Z_][a-zA-Z0-9_]*)\s*:/g, '$1"$2":');

  return JSON.parse(jsonStr);
}

export function formatHeartbeatData(data: any): HeartbeatData {
  const tee_attestation = {
    peer_id: data.tee_attestation?.peer_id,
    peer_name: data.tee_attestation?.peer_name,
    tee_quote_v4: {
      //// leave these fields as strings, display formatting is more compact.
      // header: parseStructString(data.tee_attestation?.tee_quote_v4?.header as string),
      // quote_body: parseStructString(data.tee_attestation?.tee_quote_v4?.quote_body as string),
      // signature: parseStructString(data.tee_attestation?.tee_quote_v4?.signature as string),
      header: data.tee_attestation?.tee_quote_v4?.header as string,
      quote_body: data.tee_attestation?.tee_quote_v4?.quote_body as string,
      signature: data.tee_attestation?.tee_quote_v4?.signature as string,
      signature_len: data.tee_attestation?.tee_quote_v4?.signature_len,
      time: data.tee_attestation?.tee_quote_v4?.time,
    }
  };

  return {
    ...data,
    tee_attestation,
  };
};

export const getLastSeenDiff = (time: { secs: number; nanos: number } | null) => {
  if (!time) return 'N/A';
  const now = Date.now();
  const attestationTime = (time.secs * 1000) + (time.nanos / 1_000_000);
  const diffMs = now - attestationTime;
  return formatLastSeen({
    secs: Math.floor(diffMs / 1000),
    nanos: (diffMs % 1000) * 1_000_000
  });
};