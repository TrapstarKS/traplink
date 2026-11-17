import { createHash, randomBytes, randomUUID } from 'node:crypto';
import { z } from 'zod';
import type { AppDatabase } from './database';
import type { ServerConfig } from './config';
import { ApiError } from './http';

export const registrationSchema = z.object({
  installationId: z.string().uuid(),
  name: z.string().trim().min(1).max(80).refine(value => !/[\u0000-\u001f\u007f]/.test(value)),
  platform: z.enum(['windows', 'macos']),
  appVersion: z.string().trim().max(32).default(''),
  publicKey: z.string().regex(/^[A-Za-z0-9+/]+={0,2}$/).min(40).max(128).optional(),
}).strict();
export type Registration = z.infer<typeof registrationSchema>;

interface DeviceRow {
  id: string;
  user_id: string;
  installation_id: string;
  name: string;
  platform: 'windows' | 'macos';
  app_version: string;
  public_key: string | null;
  token_hash: string;
  created_at: number;
  last_seen_at: number;
  expires_at: number;
  revoked_at: number | null;
}
interface EnrollmentRow { id: string; userId: string; expiresAt: string | number; kind: string }
interface UserRow { id: string; name: string; email: string }
export interface DevicePrincipal { deviceId: string; userId: string; tokenHash: string }

export function hashToken(token: string): string { return createHash('sha256').update(token).digest('hex'); }

export function bearerToken(request: Request): string {
  const match = /^Bearer ([A-Za-z0-9._~-]{20,256})$/i.exec(request.headers.get('authorization') ?? '');
  if (!match) throw new ApiError(401, 'unauthorized', 'Sign in from the desktop app.');
  return match[1];
}

export function deviceView(device: DeviceRow) {
  return {
    id: device.id,
    installationId: device.installation_id,
    name: device.name,
    platform: device.platform,
    appVersion: device.app_version,
    publicKey: device.public_key,
    createdAt: new Date(device.created_at).toISOString(),
    lastSeenAt: new Date(device.last_seen_at).toISOString(),
    expiresAt: new Date(device.expires_at).toISOString(),
    revokedAt: device.revoked_at === null ? null : new Date(device.revoked_at).toISOString(),
  };
}

export function requireDevice(db: AppDatabase, token: string, now = Date.now()): DevicePrincipal {
  if (!/^tlk_[A-Za-z0-9_-]{43}$/.test(token)) throw new ApiError(401, 'unauthorized', 'This device must sign in again.');
  const tokenHash = hashToken(token);
  const row = db.prepare('SELECT id, user_id FROM devices WHERE token_hash = ? AND revoked_at IS NULL AND expires_at > ?')
    .get(tokenHash, now) as { id: string; user_id: string } | undefined;
  if (!row) throw new ApiError(401, 'unauthorized', 'This device must sign in again.');
  return { deviceId: row.id, userId: row.user_id, tokenHash };
}

export function registerDevice(db: AppDatabase, token: string, input: Registration, config: ServerConfig, now = Date.now()) {
  return db.transaction(() => {
    const session = db.prepare('SELECT id, userId, expiresAt, kind FROM session WHERE token = ? AND kind = ?')
      .get(token, 'device-enrollment') as EnrollmentRow | undefined;
    if (!session || new Date(session.expiresAt).getTime() <= now) throw new ApiError(401, 'invalid_enrollment', 'Complete device authorization again.');
    const user = db.prepare('SELECT id, name, email FROM user WHERE id = ?').get(session.userId) as UserRow | undefined;
    if (!user) throw new ApiError(401, 'invalid_enrollment', 'Complete device authorization again.');
    const existing = db.prepare('SELECT * FROM devices WHERE user_id = ? AND installation_id = ?')
      .get(user.id, input.installationId) as DeviceRow | undefined;
    if (!existing) {
      const { count } = db.prepare('SELECT count(*) as count FROM devices WHERE user_id = ? AND revoked_at IS NULL').get(user.id) as { count: number };
      if (count >= 50) throw new ApiError(409, 'device_limit', 'Revoke an unused device before adding another.');
    }
    const deviceId = existing?.id ?? randomUUID();
    const deviceToken = `tlk_${randomBytes(32).toString('base64url')}`;
    const expiresAt = now + config.deviceTokenDays * 24 * 60 * 60 * 1000;
    db.prepare(`INSERT INTO devices (id, user_id, installation_id, name, platform, app_version, public_key, token_hash, created_at, last_seen_at, expires_at, revoked_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)
      ON CONFLICT(user_id, installation_id) DO UPDATE SET name=excluded.name, platform=excluded.platform,
        app_version=excluded.app_version, public_key=excluded.public_key, token_hash=excluded.token_hash,
        last_seen_at=excluded.last_seen_at, expires_at=excluded.expires_at, revoked_at=NULL`)
      .run(deviceId, user.id, input.installationId, input.name, input.platform, input.appVersion,
        input.publicKey ?? null, hashToken(deviceToken), existing?.created_at ?? now, now, expiresAt);
    const consumed = db.prepare('DELETE FROM session WHERE id = ? AND token = ? AND kind = ?').run(session.id, token, 'device-enrollment');
    if (consumed.changes !== 1) throw new ApiError(401, 'invalid_enrollment', 'Complete device authorization again.');
    const device = db.prepare('SELECT * FROM devices WHERE id = ? AND user_id = ?').get(deviceId, user.id) as DeviceRow;
    return { device: deviceView(device), deviceToken, user };
  }).immediate();
}

export function listDevices(db: AppDatabase, principal: DevicePrincipal) {
  const rows = db.prepare('SELECT * FROM devices WHERE user_id = ? ORDER BY created_at DESC').all(principal.userId) as DeviceRow[];
  return { devices: rows.map(deviceView), currentDeviceId: principal.deviceId };
}

export function heartbeat(db: AppDatabase, principal: DevicePrincipal, now = Date.now()) {
  const updated = db.prepare('UPDATE devices SET last_seen_at = ? WHERE id = ? AND user_id = ? AND token_hash = ? AND revoked_at IS NULL AND expires_at > ?')
    .run(now, principal.deviceId, principal.userId, principal.tokenHash, now);
  if (!updated.changes) throw new ApiError(401, 'unauthorized', 'This device must sign in again.');
  return { lastSeenAt: new Date(now).toISOString() };
}

export function revokeDevice(db: AppDatabase, principal: DevicePrincipal, deviceId: string, now = Date.now()) {
  const result = db.prepare('UPDATE devices SET revoked_at = COALESCE(revoked_at, ?) WHERE id = ? AND user_id = ?')
    .run(now, deviceId, principal.userId);
  if (!result.changes) throw new ApiError(404, 'not_found', 'Device not found.');
  return { revoked: true };
}
