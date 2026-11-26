import { describe, expect, it } from 'vitest';
import { randomUUID } from 'node:crypto';
import { isValidDeviceName, registrationSchema } from '../src/server/devices';
import { deviceNameValid, maxDeviceNameLength } from '../../desktop/src/lib/device-name';

function register(name: string) {
  return registrationSchema.safeParse({ installationId: randomUUID(), name, platform: 'windows', appVersion: '0.1.11' });
}

// The desktop app (Rust `model::validate_device_name` and the Preferences form) and the
// server must agree: trim like String.prototype.trim, then 1 to 80 Unicode code points
// and no control characters (Unicode category Cc).
const cases: Array<[string, boolean]> = [
  ['Work PC', true],
  ['  Work PC  ', true],
  ['\tWork PC\n', true],
  ['﻿Work PC', true],
  ['a'.repeat(80), true],
  [`${'a'.repeat(79)}🎮`, true],
  ['🖥'.repeat(80), true],
  ['a'.repeat(81), false],
  ['🖥'.repeat(81), false],
  ['', false],
  ['   ', false],
  ['　', false],
  ['Work\tPC', false],
  ['Work\u0000PC', false],
  ['Work\u007fPC', false],
  ['\u0085Work PC', false],
  ['Work\u009fPC', false],
];

describe('device name rule', () => {
  it('uses 80 code points on both sides', () => {
    expect(maxDeviceNameLength).toBe(80);
  });

  it.each(cases)('server registration accepts %j: %s', (name, valid) => {
    expect(register(name).success).toBe(valid);
    expect(isValidDeviceName(name.trim())).toBe(valid);
  });

  it.each(cases)('desktop form accepts %j: %s', (name, valid) => {
    expect(deviceNameValid(name)).toBe(valid);
  });

  it('stores the trimmed name', () => {
    const parsed = register('  Studio Mac  ');
    expect(parsed.success && parsed.data.name).toBe('Studio Mac');
  });
});
