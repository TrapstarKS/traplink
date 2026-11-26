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

export interface NamedPermissionPeer extends PermissionPeer { name: string }

const featureLabel: Record<PeerFeature, string> = { input: "mouse and keyboard", clipboard: "clipboard", files: "files" };
const featureOrder: PeerFeature[] = ["input", "clipboard", "files"];

/** Joins items as "A", "A and B" or "A, B and C". */
export function joinNames(items: string[]): string {
  return items.length <= 1 ? items.join("") : `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

/**
 * Describes what masked-grant revocations turn off, so the user is told instead of
 * finding the other computers silently disconnected. `when` picks the tense:
 * "before" for a confirmation, "after" for a notice. Returns null when nothing is revoked.
 */
export function revocationNotice(peers: NamedPermissionPeer[], changes: PermissionChange[], when: "before" | "after"): string | null {
  const lost: Record<PeerFeature, string[]> = { input: [], clipboard: [], files: [] };
  for (const change of changes) {
    const peer = peers.find(item => item.id === change.id);
    const before = permissionsOf(peer ?? { id: change.id });
    for (const feature of featureOrder) if (before[feature] && !change[feature]) lost[feature].push(peer?.name ?? change.id);
  }
  const groups = featureOrder.filter(feature => lost[feature].length).map(feature => `${featureLabel[feature]} for ${joinNames(lost[feature])}`);
  if (!groups.length) return null;
  const summary = groups.join("; ");
  const flow = lost.input.length ? ` Any saved Flow setup with ${joinNames(lost.input)} ${when === "before" ? "is" : "was"} removed.` : "";
  return `${when === "before" ? "This also turns off" : "Turned off"} ${summary}.${flow} Allow them again in Permissions if needed.`;
}
