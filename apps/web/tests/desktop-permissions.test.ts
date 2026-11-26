import { describe, expect, it } from 'vitest';
import { mergeSettingsDraft } from '../../desktop/src/lib/settings';
import { maskedGrantRevocations, needsInputGrantDisclosure } from '../../desktop/src/lib/permission-rules';

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
