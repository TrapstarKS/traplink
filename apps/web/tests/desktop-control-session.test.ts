import { describe, expect, it } from 'vitest';
import { keepaliveFailureNotice } from '../../desktop/src/lib/control-session';

describe('keepaliveFailureNotice', () => {
  it('explains why an active control session ended', () => {
    expect(keepaliveFailureNotice('Input control timed out; start it again.', { active: true, started: 3, current: 3 }))
      .toBe('The control session ended. Start control again.');
    expect(keepaliveFailureNotice('Peer is not connected', { active: true, started: 3, current: 3 }))
      .toMatch(/disconnected/);
  });

  it('stays quiet when the user already stopped or restarted control', () => {
    expect(keepaliveFailureNotice('Input control is not active', { active: false, started: 3, current: 4 })).toBeNull();
    expect(keepaliveFailureNotice('Input control is not active', { active: true, started: 3, current: 5 })).toBeNull();
  });
});
