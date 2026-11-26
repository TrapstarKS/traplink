import { messageFrom } from "./bridge";

/**
 * Message to show when the manual-control keepalive fails, or null when the
 * failure belongs to a session the user already stopped or replaced.
 */
export function keepaliveFailureNotice(error: unknown, session: { active: boolean; started: number; current: number }): string | null {
  if (!session.active || session.started !== session.current) return null;
  return messageFrom(error);
}
