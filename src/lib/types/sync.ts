export interface SyncStatus {
  enabled: boolean;
  running: boolean;
  deviceName: string | null;
  shortId: string | null;
}

export interface SyncPeer {
  id: string;
  name: string;
  lastAddr: string | null;
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
