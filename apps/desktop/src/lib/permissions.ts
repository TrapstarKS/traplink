import { nativeCommand, type ConnectedPeer, type DesktopSnapshot, type Settings } from "./bridge";

export type PeerFeature = "clipboard" | "files" | "input";
type PeerPermissions = { clipboard: boolean; files: boolean; input: boolean };

const globalSwitch: Record<PeerFeature, "clipboardEnabled" | "fileSharingEnabled" | "inputEnabled"> = {
  clipboard: "clipboardEnabled",
  files: "fileSharingEnabled",
  input: "inputEnabled",
};

export async function allowLocally(peerId: string, features: PeerFeature[]): Promise<void> {
  const snapshot = await nativeCommand<DesktopSnapshot>("get_snapshot");
  const missing = features.filter(feature => !snapshot.settings[globalSwitch[feature]]);
  if (missing.length) {
    const settings: Settings = { ...snapshot.settings };
    for (const feature of missing) settings[globalSwitch[feature]] = true;
    await nativeCommand("update_settings", { settings });
  }
  const peer = snapshot.peers.find(item => item.id === peerId);
  if (!peer) throw new Error("Unknown peer.");
  const current = permissionsOf(peer);
  const next: PeerPermissions = {
    clipboard: current.clipboard || features.includes("clipboard"),
    files: current.files || features.includes("files"),
    input: current.input || features.includes("input"),
  };
  if (next.clipboard !== current.clipboard || next.files !== current.files || next.input !== current.input) {
    await nativeCommand("set_peer_permissions", { id: peerId, ...next });
  }
}

export async function revokeForPeer(peer: ConnectedPeer, feature: PeerFeature): Promise<void> {
  await nativeCommand("set_peer_permissions", { id: peer.id, ...permissionsOf(peer), [feature]: false });
}

function permissionsOf(peer: ConnectedPeer): PeerPermissions {
  return { clipboard: !!peer.permissions?.clipboard, files: !!peer.permissions?.files, input: !!peer.permissions?.input };
}

export async function askPeerForInput(peerId: string, flow: boolean): Promise<"granted" | "outdated"> {
  try {
    await nativeCommand("request_input_consent", { peerId, flow });
    return "granted";
  } catch (error) {
    if (isOutdatedPeer(error)) return "outdated";
    throw error;
  }
}

export function isOutdatedPeer(error: unknown): boolean {
  return /older TrapLink that cannot ask/i.test(String(error instanceof Error ? error.message : error));
}

export type MacPane = "accessibility" | "input_monitoring";

export function macPermissionPanes(text: string): MacPane[] | null {
  if (!/Flow needs permission|macOS mouse and keyboard capture/i.test(text)) return null;
  const panes: MacPane[] = [];
  if (/Accessibility/i.test(text)) panes.push("accessibility");
  if (/Input Monitoring/i.test(text)) panes.push("input_monitoring");
  return panes.length ? panes : ["accessibility", "input_monitoring"];
}
