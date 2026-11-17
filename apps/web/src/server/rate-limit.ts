import { createHash } from 'node:crypto';
import { isIP } from 'node:net';
import type { AppDatabase } from './database';
import type { ServerConfig } from './config';

export function rateLimitKey(input: string): string {
  return createHash('sha256').update(input).digest('hex');
}

export function consumeLimit(db: AppDatabase, key: string, max: number, windowSeconds: number, now = Date.now()) {
  return db.transaction(() => {
    const hashedKey = rateLimitKey(key);
    const row = db.prepare('SELECT count, expires_at FROM request_limits WHERE key = ?')
      .get(hashedKey) as { count: number; expires_at: number } | undefined;
    if (!row || row.expires_at <= now) {
      db.prepare('INSERT INTO request_limits (key, count, expires_at) VALUES (?, 1, ?) ON CONFLICT(key) DO UPDATE SET count = 1, expires_at = excluded.expires_at')
        .run(hashedKey, now + windowSeconds * 1000);
      if ((now % 32) === 0) db.prepare('DELETE FROM request_limits WHERE expires_at < ?').run(now);
      return { allowed: true, retryAfter: null };
    }
    if (row.count >= max) return { allowed: false, retryAfter: Math.max(1, Math.ceil((row.expires_at - now) / 1000)) };
    db.prepare('UPDATE request_limits SET count = count + 1 WHERE key = ?').run(hashedKey);
    return { allowed: true, retryAfter: null };
  }).immediate();
}

export function requestAddress(request: Request, config: ServerConfig): string {
  const value = config.trustProxyHeaders ? request.headers.get('x-real-ip') : null;
  return value && isIP(value) ? value : 'unknown';
}
