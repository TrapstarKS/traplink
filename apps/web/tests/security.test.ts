import { afterAll, beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { randomBytes, randomUUID } from 'node:crypto';
import { getMigrations } from 'better-auth/db/migration';
import { createAuth, type TraplinkAuth } from '../src/lib/auth';
import { openDatabase, type AppDatabase } from '../src/server/database';
import { readServerConfig, type ServerConfig } from '../src/server/config';
import { hashToken, listDevices, registerDevice, registrationSchema, requireDevice, revokeDevice } from '../src/server/devices';
import { backupSchema, deleteBackup, loadBackup, saveBackup } from '../src/server/backups';
import { ApiError, assertRequestOrigin, readJson } from '../src/server/http';
import { consumeLimit } from '../src/server/rate-limit';
import { handleAuthRequest } from '../src/server/auth-handler';

const config: ServerConfig = {
  baseURL: 'http://localhost:3100',
  secret: randomBytes(48).toString('base64url'),
  databasePath: ':memory:',
  production: false,
  trustProxyHeaders: false,
  deviceTokenDays: 90,
  maxBackupBytes: 8 * 1024 * 1024,
};
let db: AppDatabase;
let auth: TraplinkAuth;
type SignedIn = { id: string; cookie: string };
let alice: SignedIn;
let bob: SignedIn;

async function authRequest(path: string, body?: unknown, cookie?: string) {
  const headers = new Headers({ origin: config.baseURL });
  if (body) headers.set('content-type', 'application/json');
  if (cookie) headers.set('cookie', cookie);
  return handleAuthRequest(new Request(`${config.baseURL}/api/auth${path}`, {
    method: body ? 'POST' : 'GET',
    headers,
    body: body ? JSON.stringify(body) : undefined,
  }), db, config, auth);
}

async function signUp(name: string): Promise<SignedIn> {
  const response = await authRequest('/sign-up/email', { name, email: `${name.toLowerCase()}@example.com`, password: 'test-password-long-enough' });
  expect(response.status).toBe(200);
  const body = await response.json();
  const cookies = response.headers.getSetCookie().map(value => value.split(';')[0]).join('; ');
  expect(cookies).toContain('session_token');
  expect(body.user.id).toBeTypeOf('string');
  return { id: body.user.id, cookie: cookies };
}

async function enroll(user: SignedIn) {
  const response = await authRequest('/device/code', { client_id: 'traplink-desktop', scope: 'devices backups' });
  expect(response.status).toBe(200);
  const code = await response.json();
  const verify = await authRequest(`/device?user_code=${code.user_code}`, undefined, user.cookie);
  expect(verify.status).toBe(200);
  expect((await verify.json()).client_id).toBe('traplink-desktop');
  const approval = await authRequest('/device/approve', { userCode: code.user_code }, user.cookie);
  expect(approval.status).toBe(200);
  const redeem = await authRequest('/device/token', {
    grant_type: 'urn:ietf:params:oauth:grant-type:device_code',
    device_code: code.device_code,
    client_id: 'traplink-desktop',
  });
  expect(redeem.status).toBe(200);
  return (await redeem.json()).access_token as string;
}

function registration(name: string) {
  return registrationSchema.parse({ installationId: randomUUID(), name, platform: 'windows', appVersion: '0.1.0' });
}

beforeAll(async () => {
  db = openDatabase(':memory:');
  auth = createAuth(db, config);
  await (await getMigrations(auth.options)).runMigrations();
  alice = await signUp('Alice');
  bob = await signUp('Bob');
});
afterAll(() => { db.close(); });
beforeEach(() => { db.prepare('DELETE FROM request_limits').run(); });

describe('real authentication and device authorization', () => {
  it('starts signup sessions immediately without email delivery and disables recovery endpoints', async () => {
    expect(auth.options.emailAndPassword?.requireEmailVerification).toBe(false);
    expect(auth.options.emailAndPassword?.autoSignIn).toBe(true);
    expect('sendResetPassword' in auth.options.emailAndPassword).toBe(false);
    expect('emailVerification' in auth.options).toBe(false);
    const production = readServerConfig({ NODE_ENV: 'production', BETTER_AUTH_SECRET: config.secret, BETTER_AUTH_URL: 'https://auth.example.com', TRUST_PROXY_HEADERS: 'true' });
    expect(production.production).toBe(true);
    for (const path of ['/request-password-reset', '/forget-password', '/reset-password', '/reset-password/unused-token', '/send-verification-email', '/verify-email']) {
      expect((await authRequest(path, { email: 'alice@example.com' })).status).toBe(404);
      expect((await authRequest(path)).status).toBe(404);
    }
  });

  it('requires the configured password length and signs in with a real persisted password', async () => {
    const short = await authRequest('/sign-up/email', { name: 'Short', email: 'short@example.com', password: '12345678' });
    expect(short.status).toBeGreaterThanOrEqual(400);
    const wrong = await authRequest('/sign-in/email', { email: 'alice@example.com', password: 'wrong-password-long' });
    expect(wrong.status).toBe(401);
    const correct = await authRequest('/sign-in/email', { email: 'alice@example.com', password: 'test-password-long-enough' });
    expect(correct.status).toBe(200);
    expect((await correct.json()).user.id).toBe(alice.id);
    const stored = db.prepare('SELECT password FROM account WHERE userId = ?').get(alice.id) as { password: string };
    expect(stored.password).not.toBe('test-password-long-enough');
  });

  it('only accepts known native clients and requires explicit authenticated approval', async () => {
    expect((await authRequest('/device/code', { client_id: 'unknown-client' })).status).toBe(400);
    expect((await authRequest('/device/code', { client_id: 'traplink-desktop', user_id: bob.id })).status).toBe(400);
    const code = await (await authRequest('/device/code', { client_id: 'traplink-desktop' })).json();
    expect((await authRequest(`/device?user_code=${code.user_code}`)).status).toBe(401);
    expect((await authRequest(`/device/?user_code=${code.user_code}`)).status).toBe(401);
    expect((await authRequest('/device/approve', { userCode: code.user_code }, alice.cookie)).status).toBe(400);
    const denied = await authRequest('/device/token', { client_id: 'traplink-desktop', device_code: code.device_code, grant_type: 'urn:ietf:params:oauth:grant-type:device_code' });
    expect(denied.status).toBe(400);
    expect((await denied.json()).error).toBe('authorization_pending');
    const claim = await authRequest(`/device?user_code=${code.user_code}`, undefined, alice.cookie);
    expect(claim.status).toBe(200);
    const secondLogin = await authRequest('/sign-in/email', { email: 'alice@example.com', password: 'test-password-long-enough' });
    const secondCookie = secondLogin.headers.getSetCookie().map(value => value.split(';')[0]).join('; ');
    const otherSession = await authRequest('/device/approve', { userCode: code.user_code }, secondCookie);
    expect(otherSession.status).toBe(403);
    const otherUser = await authRequest('/device/approve', { userCode: code.user_code }, bob.cookie);
    expect(otherUser.status).toBe(403);
    expect((await authRequest('/device/deny', { userCode: code.user_code }, alice.cookie)).status).toBe(200);
  });

  it('cannot register devices with a browser session, and consumes enrollment credentials once', async () => {
    const browserSession = db.prepare("SELECT token FROM session WHERE userId = ? AND kind = 'browser' LIMIT 1").get(alice.id) as { token: string };
    expect(() => registerDevice(db, browserSession.token, registration('Wrong route'), config)).toThrow(ApiError);
    const token = await enroll(alice);
    const session = db.prepare('SELECT kind, expiresAt FROM session WHERE token = ?').get(token) as { kind: string; expiresAt: string };
    expect(session.kind).toBe('device-enrollment');
    expect(new Date(session.expiresAt).getTime() - Date.now()).toBeLessThanOrEqual(10 * 60 * 1000);
    const result = registerDevice(db, token, registration('Alice desktop'), config);
    expect(result.user.id).toBe(alice.id);
    expect(result.deviceToken).toMatch(/^tlk_/);
    expect(() => registerDevice(db, token, registration('Replay'), config)).toThrow(ApiError);
    expect(db.prepare('SELECT id FROM session WHERE token = ?').get(token)).toBeUndefined();
    const stored = db.prepare('SELECT token_hash FROM devices WHERE id = ?').get(result.device.id) as { token_hash: string };
    expect(stored.token_hash).toBe(hashToken(result.deviceToken));
    expect(stored.token_hash).not.toContain(result.deviceToken);
  });

  it('does not accept the enrollment token as a general session, and keeps its short expiry', async () => {
    const token = await enroll(alice);
    const expiry = () => new Date((db.prepare('SELECT expiresAt FROM session WHERE token = ?').get(token) as { expiresAt: string }).expiresAt).getTime();
    const initialExpiry = expiry();
    expect(await auth.api.getSession({ headers: new Headers({ authorization: `Bearer ${token}` }) })).toBeNull();
    for (const path of ['/get-session', '/list-sessions']) {
      const response = await handleAuthRequest(new Request(`${config.baseURL}/api/auth${path}`, {
        headers: { origin: config.baseURL, authorization: `Bearer ${token}` },
      }), db, config, auth);
      expect(response.status).not.toBe(200);
      expect(await response.text()).not.toContain(alice.id);
    }
    const revoke = await handleAuthRequest(new Request(`${config.baseURL}/api/auth/revoke-sessions`, {
      method: 'POST',
      headers: { origin: config.baseURL, authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: '{}',
    }), db, config, auth);
    expect(revoke.status).not.toBe(200);
    expect((await authRequest('/get-session', undefined, alice.cookie)).status).toBe(200);
    expect(expiry()).toBe(initialExpiry);
    expect(expiry() - Date.now()).toBeLessThanOrEqual(10 * 60 * 1000);
    const result = registerDevice(db, token, registration('Alice enrolled after probing'), config);
    expect(result.user.id).toBe(alice.id);
  });

  it('removes expired device codes and unredeemed enrollment sessions without touching live ones', async () => {
    const stale = await (await authRequest('/device/code', { client_id: 'traplink-desktop' })).json();
    db.prepare("UPDATE deviceCode SET expiresAt = '2000-01-01T00:00:00.000Z' WHERE userCode = ?").run(stale.user_code);
    const abandoned = await enroll(bob);
    db.prepare("UPDATE session SET expiresAt = '2000-01-01T00:00:00.000Z' WHERE token = ?").run(abandoned);
    const browserSessions = (db.prepare("SELECT count(*) AS count FROM session WHERE kind = 'browser'").get() as { count: number }).count;
    const fresh = await (await authRequest('/device/code', { client_id: 'traplink-desktop' })).json();
    const codeCount = (userCode: string) => (db.prepare('SELECT count(*) AS count FROM deviceCode WHERE userCode = ?').get(userCode) as { count: number }).count;
    expect(codeCount(stale.user_code)).toBe(0);
    expect(codeCount(fresh.user_code)).toBe(1);
    expect(db.prepare('SELECT id FROM session WHERE token = ?').get(abandoned)).toBeUndefined();
    expect((db.prepare("SELECT count(*) AS count FROM session WHERE kind = 'browser'").get() as { count: number }).count).toBe(browserSessions);
    const result = registerDevice(db, await enroll(alice), registration('Alice after cleanup'), config);
    expect(result.user.id).toBe(alice.id);
  });

  it('reports unknown and expired user codes with the same error code', async () => {
    const code = await (await authRequest('/device/code', { client_id: 'traplink-desktop' })).json();
    const typo = `${code.user_code.slice(0, 7)}${code.user_code.endsWith('Z') ? 'Y' : 'Z'}`;
    const unknown = await authRequest(`/device?user_code=${typo}`, undefined, alice.cookie);
    expect(unknown.status).toBe(400);
    expect((await unknown.json()).error).toBe('expired_token');
    db.prepare("UPDATE deviceCode SET expiresAt = '2000-01-01T00:00:00.000Z' WHERE userCode = ?").run(code.user_code);
    const expired = await authRequest(`/device?user_code=${code.user_code}`, undefined, alice.cookie);
    expect(expired.status).toBe(400);
    expect((await expired.json()).error).toBe('expired_token');
  });
});

describe('device limit', () => {
  function insertDevices(userId: string, count: number, expiresAt: number, revokedAt: number | null = null) {
    const insert = db.prepare(`INSERT INTO devices (id, user_id, installation_id, name, platform, app_version, public_key, token_hash, created_at, last_seen_at, expires_at, revoked_at)
      VALUES (?, ?, ?, ?, 'windows', '0.1.0', NULL, ?, ?, ?, ?, ?)`);
    const ids: string[] = [];
    for (let index = 0; index < count; index++) {
      const installationId = randomUUID();
      insert.run(randomUUID(), userId, installationId, `Filler ${index}`, randomBytes(32).toString('hex'), Date.now(), Date.now(), expiresAt, revokedAt);
      ids.push(installationId);
    }
    return ids;
  }

  it('does not count expired or revoked credentials toward the limit', async () => {
    const carol = await signUp('Carol');
    insertDevices(carol.id, 50, Date.now() - 1);
    insertDevices(carol.id, 5, Date.now() + 86400_000, Date.now() - 1);
    const result = registerDevice(db, await enroll(carol), registration('Carol new computer'), config);
    expect(result.user.id).toBe(carol.id);
  });

  it('applies the limit when a revoked or expired installation signs in again', async () => {
    const dave = await signUp('Dave');
    insertDevices(dave.id, 50, Date.now() + 86400_000);
    const [revoked] = insertDevices(dave.id, 1, Date.now() + 86400_000, Date.now() - 1);
    const [expired] = insertDevices(dave.id, 1, Date.now() - 1);
    for (const installationId of [revoked, expired]) {
      const input = registrationSchema.parse({ installationId, name: 'Old computer', platform: 'windows', appVersion: '0.1.0' });
      const token = await enroll(dave);
      expect(() => registerDevice(db, token, input, config)).toThrow('Revoke an unused device');
    }
    const [live] = (db.prepare('SELECT installation_id FROM devices WHERE user_id = ? AND revoked_at IS NULL AND expires_at > ? LIMIT 1').all(dave.id, Date.now()) as { installation_id: string }[]).map(row => row.installation_id);
    const rotated = registerDevice(db, await enroll(dave), registrationSchema.parse({ installationId: live, name: 'Still here', platform: 'windows', appVersion: '0.1.0' }), config);
    expect(rotated.user.id).toBe(dave.id);
    expect((db.prepare('SELECT count(*) AS count FROM devices WHERE user_id = ? AND revoked_at IS NULL AND expires_at > ?').get(dave.id, Date.now()) as { count: number }).count).toBe(50);
  });
});

describe('tenant isolation and credential revocation', () => {
  it('keeps devices and backups private to their owner, including guessed IDs', async () => {
    const aliceDevice = registerDevice(db, await enroll(alice), registration('Alice second computer'), config);
    const bobDevice = registerDevice(db, await enroll(bob), registration('Bob computer'), config);
    const principalA = requireDevice(db, aliceDevice.deviceToken);
    const principalB = requireDevice(db, bobDevice.deviceToken);
    expect(listDevices(db, principalA).devices.every(device => device.id !== bobDevice.device.id)).toBe(true);
    expect(listDevices(db, principalB).devices.map(device => device.id)).toEqual([bobDevice.device.id]);
    expect(() => revokeDevice(db, principalA, bobDevice.device.id)).toThrow('Device not found');
    const envelope = backupSchema.parse({
      version: 1, algorithm: 'xchacha20poly1305',
      salt: randomBytes(16).toString('base64'), nonce: randomBytes(24).toString('base64'), ciphertext: randomBytes(32).toString('base64'),
    });
    saveBackup(db, principalA, envelope, config.maxBackupBytes);
    expect(() => loadBackup(db, principalB)).toThrow('No cloud backup');
    deleteBackup(db, principalB);
    expect(loadBackup(db, principalA).backup).toEqual(envelope);
    revokeDevice(db, principalA, aliceDevice.device.id);
    expect(() => requireDevice(db, aliceDevice.deviceToken)).toThrow('sign in again');
    expect(requireDevice(db, bobDevice.deviceToken).userId).toBe(bob.id);
  });

  it('rotates credentials on repeated authorization of the same installation', async () => {
    const input = registration('Same installation');
    const first = registerDevice(db, await enroll(alice), input, config);
    const second = registerDevice(db, await enroll(alice), input, config);
    expect(second.device.id).toBe(first.device.id);
    expect(second.deviceToken).not.toBe(first.deviceToken);
    expect(() => requireDevice(db, first.deviceToken)).toThrow(ApiError);
    expect(requireDevice(db, second.deviceToken).userId).toBe(alice.id);
    expect(() => requireDevice(db, second.deviceToken, Date.now() + 366 * 86400_000)).toThrow(ApiError);
  });

  it('rejects missing credentials, malformed envelopes, oversized requests, and foreign origins', async () => {
    expect(() => requireDevice(db, '')).toThrow(ApiError);
    expect(() => assertRequestOrigin(new Request(config.baseURL, { headers: { origin: 'https://other.example' } }), config)).toThrow(ApiError);
    expect(() => readServerConfig({ NODE_ENV: 'production', BETTER_AUTH_SECRET: config.secret, BETTER_AUTH_URL: 'http://localhost:3100' })).toThrow('HTTPS');
    expect(() => readServerConfig({ NODE_ENV: 'test', BETTER_AUTH_SECRET: 'GENERATE_A_RANDOM_SECRET_AT_LEAST_32_CHARACTERS' })).toThrow('randomly generated');
    expect(() => readServerConfig({ NODE_ENV: 'production', BETTER_AUTH_SECRET: config.secret, BETTER_AUTH_URL: 'https://auth.example.com', TRUST_PROXY_HEADERS: 'true' })).not.toThrow();
    const request = new Request(config.baseURL, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ field: 'x'.repeat(1024) }) });
    await expect(readJson(request, backupSchema, 100)).rejects.toMatchObject({ status: 413 });
    expect(backupSchema.safeParse({ version: 1, ciphertext: 'plaintext' }).success).toBe(false);
    expect(consumeLimit(db, 'test-strict-limit', 2, 60, 1000).allowed).toBe(true);
    expect(consumeLimit(db, 'test-strict-limit', 2, 60, 1000).allowed).toBe(true);
    expect(consumeLimit(db, 'test-strict-limit', 2, 60, 1000).allowed).toBe(false);
    expect(consumeLimit(db, 'test-strict-limit', 2, 60, 61_001).allowed).toBe(true);
  });

  it('validates a maximum-size native ciphertext without regexp stack exhaustion', () => {
    const ciphertext = Buffer.alloc(5 * 1024 * 1024 + 16).toString('base64');
    const envelope = {
      version: 1, algorithm: 'xchacha20poly1305',
      salt: Buffer.alloc(16).toString('base64'), nonce: Buffer.alloc(24).toString('base64'), ciphertext,
    };
    expect(backupSchema.safeParse(envelope).success).toBe(true);
    expect(backupSchema.safeParse({ ...envelope, ciphertext: `${ciphertext.slice(0, -4)}?AAA` }).success).toBe(false);
  });
});
