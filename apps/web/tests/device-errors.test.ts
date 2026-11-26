import { describe, expect, it } from 'vitest';
import { deviceError } from '../src/lib/device-errors';

describe('device authorization error copy', () => {
  it('does not tell a user with a mistyped code that it expired', () => {
    const message = deviceError({ error: 'expired_token', status: 400 }, 'fallback');
    expect(message).toContain('invalid or has expired');
    expect(message).toContain('Check each character');
  });

  it('offers a way forward when the code was started in another session', () => {
    expect(deviceError({ status: 403 }, 'fallback')).toContain('generate a new code');
    expect(deviceError({ error: 'access_denied', status: 403 }, 'fallback')).toContain('another browser session or account');
  });

  it('keeps the other mappings', () => {
    expect(deviceError({ status: 429 }, 'fallback')).toContain('Too many attempts');
    expect(deviceError({ status: 401 }, 'fallback')).toContain('Sign in again');
    expect(deviceError({ status: 503 }, 'fallback')).toContain('unavailable');
    expect(deviceError({ status: 400 }, 'fallback')).toBe('fallback');
    expect(deviceError(null, 'fallback')).toBe('fallback');
  });
});
