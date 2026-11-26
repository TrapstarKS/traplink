export type Platform = "windows" | "macos" | "linux" | "unknown";
export interface Settings { deviceName: string; authServerUrl: string; clipboardEnabled: boolean; fileSharingEnabled: boolean; inputEnabled: boolean; startOnLogin: boolean; theme: "dark" | "light" | "system"; downloadDirectory: string; port: number; remapEnabled?: boolean; }
export interface Device { id: string; name: string; vendorId: number; productId: number; transport: string; serialNumber?: string; manufacturer?: string; isLogitech: boolean; capabilities: string[]; inputKinds: Array<"keyboard" | "mouse">; }
export interface Peer { id: string; name: string; address: string; platform: Platform; fingerprint: string; connected: boolean; trusted: boolean; lastSeen?: string; }
export interface KeyMapping { from: string; to: string; }
export interface Profile { id: string; name: string; application?: string; deviceId?: string; mappings: KeyMapping[]; createdAt: string; updatedAt: string; }
export interface Backup { id: string; name: string; createdAt: string; profileCount: number; }
export interface Transfer { id: string; fileName: string; peerName: string; direction: "incoming" | "outgoing"; bytes: number; status: string; createdAt: string; }
export interface Snapshot { deviceId: string; deviceName: string; platform: Platform; version: string; settings: Settings; devices: Device[]; peers: Peer[]; profiles: Profile[]; backups: Backup[]; transfers: Transfer[]; listening: boolean; localAddresses: string[]; pairingCode?: string; account?: {name: string; email: string}; accountExpired?: boolean; permissions: {accessibility: boolean; inputMonitoring: boolean}; }
export interface PairingInvite { invitation: string; code: string; address: string; expiresAt: string; fingerprint: string; }
export { deviceNameValid, maxDeviceNameLength } from "./device-name";
