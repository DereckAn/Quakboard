export interface SyncStatus {
  enabled: boolean;
  running: boolean;
  deviceName: string | null;
  shortId: string | null;
  /** This device's LAN IPs, to type on the other device. */
  addresses: string[];
  /** The "Also sync images" setting. */
  syncImages: boolean;
  /** Largest image that syncs, from the backend's own limit. */
  maxImageBytes: number;
}

export interface SyncPeer {
  id: string;
  name: string;
  lastAddr: string | null;
  /** Seen on the network right now. */
  isOnline: boolean;
}

export interface NearbyDevice {
  id: string;
  shortId: string;
  addr: string;
}

/** Payload of the `sync-pairing` event. */
export type PairingEvent =
  | { kind: "codeShown"; code: string }
  | { kind: "paired"; peerId: string; name: string }
  | { kind: "failed"; reason: string };

export interface SendFileResult {
  /** Chosen devices that got the offer right now. */
  reached: number;
  chosen: number;
}

/** Payload of the `sync-file-progress` event. */
export interface FileProgress {
  itemId: string;
  phase: "hashing" | "fetching";
  done: number;
  total: number;
}

/** Payload of the `sync-file-done` event. */
export interface FileDone {
  itemId: string;
  ok: boolean;
  cancelled: boolean;
  error: string | null;
}

/** `content_metadata.remote` of a file another device offered. */
export interface RemoteFileInfo {
  offer_id: string;
  origin_device_id: string;
  origin_name: string;
  sha256: string;
  size: number;
  mime: string;
}
