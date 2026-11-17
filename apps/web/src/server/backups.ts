import { createHash } from 'node:crypto';
import { z } from 'zod';
import type { AppDatabase } from './database';
import type { DevicePrincipal } from './devices';
import { ApiError } from './http';

function isBase64(value: string): boolean {
  if (!value.length || value.length % 4 !== 0) return false;
  const padding = value.endsWith('==') ? 2 : value.endsWith('=') ? 1 : 0;
  const dataLength = value.length - padding;
  for (let index = 0; index < dataLength; index++) {
    const char = value.charCodeAt(index);
    if (!(char >= 65 && char <= 90 || char >= 97 && char <= 122 || char >= 48 && char <= 57 || char === 43 || char === 47)) return false;
  }
  return true;
}
const base64 = z.string().refine(isBase64);
export const backupSchema = z.object({
  version: z.literal(1),
  algorithm: z.literal('xchacha20poly1305'),
  salt: base64.min(24).max(88),
  nonce: base64.length(32),
  ciphertext: base64.min(24),
}).strict();
export type BackupEnvelope = z.infer<typeof backupSchema>;
interface BackupRow { payload: string; sha256: string; bytes: number; updated_at: number; updated_by: string }

export function saveBackup(db: AppDatabase, principal: DevicePrincipal, envelope: BackupEnvelope, maxBytes: number, now = Date.now()) {
  const payload = JSON.stringify(envelope);
  const bytes = Buffer.byteLength(payload, 'utf8');
  if (bytes > maxBytes) throw new ApiError(413, 'payload_too_large', 'The backup is too large.');
  if (Buffer.from(envelope.salt, 'base64').length < 16 || Buffer.from(envelope.nonce, 'base64').length !== 24 || Buffer.from(envelope.ciphertext, 'base64').length < 16) {
    throw new ApiError(400, 'invalid_backup', 'The encrypted backup envelope is invalid.');
  }
  const sha256 = createHash('sha256').update(payload).digest('hex');
  db.prepare(`INSERT INTO encrypted_backups (user_id, payload, sha256, bytes, updated_at, updated_by) VALUES (?, ?, ?, ?, ?, ?)
    ON CONFLICT(user_id) DO UPDATE SET payload=excluded.payload, sha256=excluded.sha256, bytes=excluded.bytes,
      updated_at=excluded.updated_at, updated_by=excluded.updated_by`)
    .run(principal.userId, payload, sha256, bytes, now, principal.deviceId);
  return { updatedAt: new Date(now).toISOString(), sha256, bytes };
}

export function loadBackup(db: AppDatabase, principal: DevicePrincipal) {
  const row = db.prepare('SELECT payload, sha256, bytes, updated_at, updated_by FROM encrypted_backups WHERE user_id = ?')
    .get(principal.userId) as BackupRow | undefined;
  if (!row) throw new ApiError(404, 'not_found', 'No cloud backup has been saved for this account.');
  return { backup: JSON.parse(row.payload) as BackupEnvelope, updatedAt: new Date(row.updated_at).toISOString(), sha256: row.sha256, bytes: row.bytes, updatedBy: row.updated_by };
}

export function deleteBackup(db: AppDatabase, principal: DevicePrincipal) {
  db.prepare('DELETE FROM encrypted_backups WHERE user_id = ?').run(principal.userId);
  return { deleted: true };
}
