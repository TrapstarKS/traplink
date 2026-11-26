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
 */
export function inviteState(invite: { code: string; expiresAt: string }, pairingCode: string | null | undefined, peers: readonly PeerLike[], baseline: InviteBaseline, now: number): InviteState {
  const live = now < Date.parse(invite.expiresAt);
  if (pairingCode === invite.code) return live ? { kind: "available" } : { kind: "expired" };
  const paired = peers.find(peer => !baseline.known.includes(peer.id))
    ?? peers.find(peer => peer.connected && !baseline.connected.includes(peer.id));
  if (paired) return { kind: "paired", peer: { id: paired.id, name: paired.name } };
  return live ? { kind: "closed" } : { kind: "expired" };
}
