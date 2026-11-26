import { z } from 'zod';
import type { TraplinkAuth } from '../lib/auth';
import type { AppDatabase } from './database';
import type { ServerConfig } from './config';
import { ApiError, assertRequestOrigin, json, limitRequest, readJson } from './http';

const approvalBody = z.object({ userCode: z.string().min(1).max(191) });
const codeBody = z.object({ client_id: z.literal('traplink-desktop'), scope: z.string().max(64).optional() }).strict();

async function guardBrowserApproval(request: Request, db: AppDatabase, auth: TraplinkAuth, path: string) {
  if (request.headers.has('authorization')) throw new ApiError(400, 'invalid_request', 'Use the signed-in browser to authorize a device.');
  const cookie = request.headers.get('cookie');
  if (!cookie) throw new ApiError(401, 'unauthorized', 'Sign in before authorizing a device.');
  const session = await auth.api.getSession({ headers: new Headers({ cookie }) });
  if (!session) throw new ApiError(401, 'unauthorized', 'Sign in before authorizing a device.');
  const row = db.prepare('SELECT kind FROM session WHERE id = ? AND userId = ?').get(session.session.id, session.user.id) as { kind: string } | undefined;
  if (row?.kind !== 'browser') throw new ApiError(401, 'unauthorized', 'A browser sign-in is required.');
  const code = path === '/device'
    ? new URL(request.url).searchParams.get('user_code') ?? ''
    : (await readJson(request.clone(), approvalBody)).userCode;
  const userCode = code.replace(/[^a-zA-Z0-9]/g, '').toUpperCase();
  if (userCode.length !== 8) throw new ApiError(400, 'invalid_request', 'Enter the eight-character code shown on your desktop.');
  db.transaction(() => {
    const deviceCode = db.prepare('SELECT id, userId, expiresAt, status FROM deviceCode WHERE userCode = ?').get(userCode) as {
      id: string; userId: string | null; expiresAt: string | number; status: string;
    } | undefined;
    if (!deviceCode || new Date(deviceCode.expiresAt).getTime() <= Date.now()) throw new ApiError(400, 'expired_token', 'This code is invalid or expired.');
    if (deviceCode.userId && deviceCode.userId !== session.user.id) throw new ApiError(403, 'access_denied', 'This code belongs to a different sign-in.');
    const claim = db.prepare('SELECT session_id FROM device_approval_claims WHERE code_id = ?').get(deviceCode.id) as { session_id: string } | undefined;
    if (claim && claim.session_id !== session.session.id) throw new ApiError(403, 'access_denied', 'Continue in the browser where this code was first entered.');
    if (path !== '/device' && !claim) throw new ApiError(400, 'invalid_request', 'Verify the device code before approving or denying it.');
    if (path === '/device' && !claim) {
      db.prepare('DELETE FROM device_approval_claims WHERE expires_at < ?').run(Date.now());
      db.prepare('INSERT INTO device_approval_claims (code_id, session_id, user_id, expires_at) VALUES (?, ?, ?, ?)')
        .run(deviceCode.id, session.session.id, session.user.id, new Date(deviceCode.expiresAt).getTime());
    }
  }).immediate();
}

// Dates written by Better Auth are ISO strings in SQLite. Compare by storage type so a numeric value
// is never ordered against text (SQLite sorts every INTEGER below every TEXT).
const EXPIRED = `((typeof(expiresAt) = 'text' AND expiresAt < @iso) OR (typeof(expiresAt) IN ('integer', 'real') AND expiresAt < @ms))`;

export function sweepExpiredDeviceAuthorization(db: AppDatabase, now = Date.now()) {
  const params = { iso: new Date(now).toISOString(), ms: now };
  db.transaction(() => {
    db.prepare(`DELETE FROM deviceCode WHERE ${EXPIRED}`).run(params);
    db.prepare(`DELETE FROM session WHERE kind = 'device-enrollment' AND ${EXPIRED}`).run(params);
    db.prepare('DELETE FROM device_approval_claims WHERE expires_at < ?').run(now);
  })();
}

export async function handleAuthRequest(request: Request, db: AppDatabase, config: ServerConfig, auth: TraplinkAuth): Promise<Response> {
  try {
    assertRequestOrigin(request, config);
    limitRequest(db, request, config, 'auth-outer', 100, 60);
    const pathname = new URL(request.url).pathname;
    if (/%|\\|\/\//.test(pathname)) throw new ApiError(400, 'invalid_request', 'The authentication path is invalid.');
    const path = pathname.replace(/^\/api\/auth/, '').replace(/\/+$/, '');
    if (/^\/(?:request-password-reset|forget-password|reset-password|send-verification-email|verify-email)(?:\/|$)/.test(path)) {
      return json({ error: 'not_found', message: 'Endpoint not found.' }, 404);
    }
    if (/^bearer\s/i.test(request.headers.get('authorization') ?? '')) {
      throw new ApiError(400, 'invalid_request', 'Bearer credentials are not accepted here.');
    }
    if (request.method === 'POST' && path === '/device/code') {
      const body = await readJson(request.clone(), codeBody);
      if (body.scope?.split(/\s+/).some(scope => scope && !['devices', 'backups'].includes(scope))) {
        throw new ApiError(400, 'invalid_scope', 'Only device and backup access can be requested.');
      }
      sweepExpiredDeviceAuthorization(db);
    }
    if ((request.method === 'GET' && path === '/device') ||
        (request.method === 'POST' && ['/device/approve', '/device/deny'].includes(path))) {
      await guardBrowserApproval(request, db, auth, path);
    }
    const response = await auth.handler(request);
    response.headers.set('Cache-Control', 'no-store');
    response.headers.set('Pragma', 'no-cache');
    return response;
  } catch (error) {
    if (error instanceof ApiError) return json({ error: error.code, message: error.message }, error.status);
    return json({ error: 'service_unavailable', message: 'Authentication is temporarily unavailable.' }, 503);
  }
}
