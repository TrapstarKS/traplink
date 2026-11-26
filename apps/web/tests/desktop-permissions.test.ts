import { describe, expect, it } from 'vitest';
import { mergeSettingsDraft } from '../../desktop/src/lib/settings';
import { joinNames, maskedGrantRevocations, needsInputGrantDisclosure, revocationNotice } from '../../desktop/src/lib/permission-rules';
import { singleFlight } from '../../desktop/src/lib/single-flight';

type Settings = Parameters<typeof mergeSettingsDraft>[0];

const settings = (patch: Partial<Settings> = {}): Settings => ({
  deviceName: 'A',
  authServerUrl: 'https://traplink.example',
  clipboardEnabled: false,
  fileSharingEnabled: false,
  inputEnabled: false,
  startOnLogin: false,
  theme: 'dark',
  downloadDirectory: 'C:/Downloads',
  port: 48372,
  remapEnabled: false,
  ...patch,
});

describe('mergeSettingsDraft', () => {
  it('keeps an emergency stop that happened while the draft was dirty', () => {
    const merged = mergeSettingsDraft(
      settings({ inputEnabled: false }),
      settings({ inputEnabled: true }),
      settings({ inputEnabled: true, deviceName: 'B' }),
    );
    expect(merged.inputEnabled).toBe(false);
    expect(merged.deviceName).toBe('B');
  });

  it('keeps background grants the user did not touch', () => {
    const merged = mergeSettingsDraft(settings({ clipboardEnabled: true }), settings(), settings({ theme: 'light' }));
    expect(merged.clipboardEnabled).toBe(true);
    expect(merged.theme).toBe('light');
  });

  it('applies switches the user changed and keeps remapping as it is', () => {
    const merged = mergeSettingsDraft(settings({ remapEnabled: true }), settings(), settings({ inputEnabled: true }));
    expect(merged.inputEnabled).toBe(true);
    expect(merged.remapEnabled).toBe(true);
  });
});

describe('needsInputGrantDisclosure', () => {
  const peer = (input: boolean) => ({ id: 'b', permissions: { clipboard: false, files: false, input } });
  it('asks before a grant that does not exist yet', () => {
    expect(needsInputGrantDisclosure(peer(false), { inputEnabled: true })).toBe(true);
    expect(needsInputGrantDisclosure(peer(true), { inputEnabled: false })).toBe(true);
    expect(needsInputGrantDisclosure(undefined, { inputEnabled: true })).toBe(true);
  });
  it('does not ask when the grant is already active', () => {
    expect(needsInputGrantDisclosure(peer(true), { inputEnabled: true })).toBe(false);
  });
});

describe('maskedGrantRevocations', () => {
  const peers = [
    { id: 'a', permissions: { clipboard: true, files: true, input: true } },
    { id: 'b', permissions: { clipboard: false, files: false, input: true } },
    { id: 'c', permissions: { clipboard: false, files: false, input: false } },
  ];

  it('revokes other computers when a feature for one computer turns the switch on', () => {
    expect(maskedGrantRevocations(peers, 'b', settings(), ['input'])).toEqual([
      { id: 'a', clipboard: true, files: true, input: false },
    ]);
    expect(maskedGrantRevocations(peers, 'c', settings({ fileSharingEnabled: true }), ['clipboard', 'files'])).toEqual([
      { id: 'a', clipboard: false, files: true, input: true },
    ]);
  });

  it('leaves other computers alone when the switch is already on', () => {
    expect(maskedGrantRevocations(peers, 'b', settings({ inputEnabled: true }), ['input'])).toEqual([]);
  });
});

describe('revocationNotice', () => {
  const peers = [
    { id: 'a', name: 'Laptop', permissions: { clipboard: true, files: true, input: true } },
    { id: 'b', name: 'Tablet', permissions: { clipboard: false, files: false, input: true } },
    { id: 'c', name: 'Desk', permissions: { clipboard: true, files: false, input: false } },
  ];

  it('names every computer and feature a grant turns off', () => {
    const changes = maskedGrantRevocations(peers, 'b', settings(), ['input', 'clipboard']);
    expect(revocationNotice(peers, changes, 'before')).toBe(
      'This also turns off mouse and keyboard for Laptop; clipboard for Laptop and Desk. Any saved Flow setup with Laptop is removed. Allow them again in Permissions if needed.',
    );
    expect(revocationNotice(peers, maskedGrantRevocations(peers, 'a', settings(), ['input']), 'after')).toBe(
      'Turned off mouse and keyboard for Tablet. Any saved Flow setup with Tablet was removed. Allow them again in Permissions if needed.',
    );
    expect(revocationNotice(peers, maskedGrantRevocations(peers, 'b', settings(), ['clipboard']), 'after')).toBe(
      'Turned off clipboard for Laptop and Desk. Allow them again in Permissions if needed.',
    );
  });

  it('says nothing when no other computer loses access', () => {
    expect(revocationNotice(peers, maskedGrantRevocations(peers, 'b', settings({ inputEnabled: true }), ['input']), 'after')).toBeNull();
    expect(joinNames(['A', 'B', 'C'])).toBe('A, B and C');
  });
});

describe('singleFlight', () => {
  it('ignores a second call while the first is still running', async () => {
    const run = singleFlight();
    let calls = 0;
    let release = () => {};
    const task = () => { calls += 1; return new Promise<string>(resolve => { release = () => resolve('done'); }); };
    const first = run(task);
    const second = run(task);
    expect(calls).toBe(1);
    expect(await second).toBeUndefined();
    release();
    expect(await first).toBe('done');
    const third = run(async () => { calls += 1; return 'again'; });
    expect(await third).toBe('again');
    expect(calls).toBe(2);
  });

  it('accepts new calls after a task fails', async () => {
    const run = singleFlight();
    await expect(run(async () => { throw new Error('boom'); })).rejects.toThrow('boom');
    expect(await run(async () => 'ok')).toBe('ok');
  });
});
