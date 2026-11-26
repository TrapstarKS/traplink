type PeerLike = { id: string; name: string; connected: boolean };

/** Peers known when an invite was created, used to recognise the computer that pairs with it. */
export interface InviteBaseline { known: string[]; connected: string[] }

export type InviteState =
  | { kind: "available" }
  | { kind: "paired"; peer: { id: string; name: string } }
  | { kind: "expired" }
  | { kind: "closed" };

export function inviteBaseline(peers: readonly PeerLike[] | undefined): InviteBaseline {
  return { known: (peers ?? []).map(peer => peer.id), connected: (peers ?? []).filter(peer => peer.connected).map(peer => peer.id) };
}

/**
 * State of an invite created on this computer. The core drops the invite as
 * soon as another computer completes the pairing handshake, so a consumed
 * invite plus a computer that is new (or newly connected) means it paired.
 *
 * The core also drops the invite from the snapshot once it expires, so after
 * the deadline only a computer that was not known before counts as paired. A
 * known computer reconnecting (for example after waking from sleep) does not.
 */
export function inviteState(invite: { code: string; expiresAt: string }, pairingCode: string | null | undefined, peers: readonly PeerLike[], baseline: InviteBaseline, now: number): InviteState {
  const live = now < Date.parse(invite.expiresAt);
  if (pairingCode === invite.code) return live ? { kind: "available" } : { kind: "expired" };
  const added = peers.find(peer => !baseline.known.includes(peer.id));
  if (added) return { kind: "paired", peer: { id: added.id, name: added.name } };
  if (!live) return { kind: "expired" };
  const repaired = peers.find(peer => peer.connected && !baseline.connected.includes(peer.id));
  if (repaired) return { kind: "paired", peer: { id: repaired.id, name: repaired.name } };
  return { kind: "closed" };
}
