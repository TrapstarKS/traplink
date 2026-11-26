import type { Settings } from "@traplink/shared";

// Keep this file free of Tauri imports: it is unit tested under Node.

export type PeerFeature = "clipboard" | "files" | "input";
export type PeerPermissions = { clipboard: boolean; files: boolean; input: boolean };
export interface PermissionPeer { id: string; permissions?: { clipboard: boolean; files: boolean; input?: boolean } }
export type PermissionChange = PeerPermissions & { id: string };

export const globalSwitch: Record<PeerFeature, "clipboardEnabled" | "fileSharingEnabled" | "inputEnabled"> = {
  clipboard: "clipboardEnabled",
  files: "fileSharingEnabled",
  input: "inputEnabled",
};

export function permissionsOf(peer: PermissionPeer): PeerPermissions {
  return { clipboard: !!peer.permissions?.clipboard, files: !!peer.permissions?.files, input: !!peer.permissions?.input };
}

/** Starting Flow or control also lets the peer control this computer; disclose it when that grant is not active yet. */
export function needsInputGrantDisclosure(peer: PermissionPeer | undefined, settings: Pick<Settings, "inputEnabled">): boolean {
  return !peer?.permissions?.input || !settings.inputEnabled;
}

/**
 * When enabling a feature for one computer turns its global switch on, the grants
 * other computers kept while it was off would silently come back. Returns the
 * permission updates that revoke them.
 */
export function maskedGrantRevocations(peers: PermissionPeer[], peerId: string, settings: Settings, features: PeerFeature[]): PermissionChange[] {
  const turningOn = features.filter(feature => !settings[globalSwitch[feature]]);
  const changes: PermissionChange[] = [];
  for (const peer of peers) {
    if (peer.id === peerId) continue;
    const next = permissionsOf(peer);
    const masked = turningOn.filter(feature => next[feature]);
    if (!masked.length) continue;
    for (const feature of masked) next[feature] = false;
    changes.push({ id: peer.id, ...next });
  }
  return changes;
}
