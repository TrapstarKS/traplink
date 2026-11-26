import { betterAuth } from 'better-auth';
import { deviceAuthorization } from 'better-auth/plugins';
import type { AppDatabase } from '../server/database';
import { getDatabase } from '../server/database';
import { readServerConfig, type ServerConfig } from '../server/config';
import { consumeLimit } from '../server/rate-limit';

export const DESKTOP_CLIENT_ID = 'traplink-desktop';

export function createAuth(db: AppDatabase, config: ServerConfig) {
  return betterAuth({
    appName: 'TrapLink',
    baseURL: config.baseURL,
    basePath: '/api/auth',
    secret: config.secret,
    database: db,
    trustedOrigins: [config.baseURL],
    logger: { disabled: true },
    advanced: {
      useSecureCookies: config.production || config.baseURL.startsWith('https:'),
      ipAddress: { ipAddressHeaders: config.trustProxyHeaders ? ['x-real-ip'] : ['x-traplink-no-client-ip'] },
      defaultCookieAttributes: { httpOnly: true, sameSite: 'lax' },
    },
    emailAndPassword: {
      enabled: true,
      minPasswordLength: 12,
      maxPasswordLength: 128,
      requireEmailVerification: false,
      autoSignIn: true,
    },
    session: {
      expiresIn: 7 * 24 * 60 * 60,
      updateAge: 24 * 60 * 60,
      cookieCache: { enabled: false },
      additionalFields: {
        kind: { type: 'string', required: false, defaultValue: 'browser', input: false, returned: false },
      },
    },
    databaseHooks: {
      session: { create: { before: async (session, context) => ({ data: {
        ...session,
        kind: context?.path === '/device/token' ? 'device-enrollment' : 'browser',
        ...(context?.path === '/device/token' ? { expiresAt: new Date(Date.now() + 10 * 60 * 1000) } : {}),
      } }) } },
    },
    rateLimit: {
      enabled: true,
      max: 60,
      window: 60,
      customStorage: { consume: async (key, rule) => consumeLimit(db, `auth:${key}`, rule.max, rule.window) },
      customRules: {
        '/sign-in/email': { window: 60, max: 8 },
        '/sign-up/email': { window: 300, max: 5 },
        '/device/code': { window: 60, max: 5 },
        '/device/token': { window: 60, max: 30 },
      },
    },
    // No bearer() plugin: the device enrollment token is only redeemed by /api/v1/devices/register,
    // and must never act as a general session (list/extend/revoke sessions, update the user).
    plugins: [
      deviceAuthorization({
        verificationUri: `${config.baseURL}/device`,
        validateClient: clientId => clientId === DESKTOP_CLIENT_ID,
        expiresIn: '10m',
        interval: '5s',
      }),
    ],
  });
}

export type TraplinkAuth = ReturnType<typeof createAuth>;
const globalAuth = globalThis as typeof globalThis & { __traplinkAuth?: TraplinkAuth };
export function getAuth(): TraplinkAuth {
  if (!globalAuth.__traplinkAuth) globalAuth.__traplinkAuth = createAuth(getDatabase(), readServerConfig());
  return globalAuth.__traplinkAuth;
}

export const auth: TraplinkAuth = new Proxy({} as TraplinkAuth, {
  get(_target, property) { return Reflect.get(getAuth(), property); },
});
