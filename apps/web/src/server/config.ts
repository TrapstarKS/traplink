import { isAbsolute, resolve } from 'node:path';

export interface ServerConfig {
  baseURL: string;
  secret: string;
  databasePath: string;
  production: boolean;
  trustProxyHeaders: boolean;
  deviceTokenDays: number;
  maxBackupBytes: number;
}

export function readServerConfig(env: NodeJS.ProcessEnv = process.env): ServerConfig {
  const production = env.NODE_ENV === 'production';
  const secret = env.BETTER_AUTH_SECRET ?? '';
  if (secret.length < 32 || /^(change|replace|example|generate|your[-_])/i.test(secret)) {
    throw new Error('BETTER_AUTH_SECRET must contain at least 32 randomly generated characters.');
  }
  let base: URL;
  try {
    base = new URL(env.BETTER_AUTH_URL ?? 'http://localhost:3100');
  } catch {
    throw new Error('BETTER_AUTH_URL must be an absolute URL.');
  }
  const localHost = ['localhost', '127.0.0.1', '[::1]'].includes(base.hostname);
  if (base.username || base.password || base.search || base.hash || base.pathname !== '/') {
    throw new Error('BETTER_AUTH_URL must be an origin without credentials, query, or path.');
  }
  if (base.protocol !== 'https:' && !(base.protocol === 'http:' && localHost && !production)) {
    throw new Error('Authentication requires HTTPS except on localhost in development.');
  }
  const trustProxyHeaders = env.TRUST_PROXY_HEADERS === 'true';
  if (production && !trustProxyHeaders) {
    throw new Error('Production requires TRUST_PROXY_HEADERS=true and a proxy that overwrites X-Real-IP.');
  }
  const databaseValue = env.DATABASE_PATH ?? './data/traplink.sqlite';
  const databasePath = databaseValue === ':memory:' || isAbsolute(databaseValue)
    ? databaseValue
    : resolve(process.cwd(), databaseValue);
  const deviceTokenDays = boundedInteger(env.DEVICE_TOKEN_DAYS, 90, 1, 365, 'DEVICE_TOKEN_DAYS');
  const maxBackupBytes = boundedInteger(env.MAX_BACKUP_BYTES, 8 * 1024 * 1024, 1024, 32 * 1024 * 1024, 'MAX_BACKUP_BYTES');

  return { baseURL: base.origin, secret, databasePath, production, trustProxyHeaders, deviceTokenDays, maxBackupBytes };
}

function boundedInteger(value: string | undefined, fallback: number, min: number, max: number, name: string): number {
  const parsed = value === undefined ? fallback : Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < min || parsed > max) {
    throw new Error(`${name} is outside its supported range.`);
  }
  return parsed;
}
