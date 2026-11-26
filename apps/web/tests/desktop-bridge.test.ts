import { describe, expect, it } from 'vitest';
import { messageFrom } from '../../desktop/src/lib/bridge';

describe('desktop messageFrom', () => {
  it('keeps the firewall hint for LAN connection timeouts', () => {
    const text = messageFrom('Connection timed out. Confirm both computers are on the same network and the firewall allows TrapLink.');
    expect(text).toMatch(/firewall/i);
    expect(text).not.toMatch(/device didn't respond/i);
    expect(messageFrom('Connection attempt timed out.')).toMatch(/firewall/i);
  });

  it('tells the user to restart an expired control session', () => {
    expect(messageFrom('Input control timed out; start it again.')).toBe('The control session ended. Start control again.');
  });

  it('explains an unconfirmed Flow handoff', () => {
    expect(messageFrom('timeout: the other computer did not confirm Flow.')).toMatch(/didn't confirm Flow/i);
  });

  it('still maps device timeouts to the device message', () => {
    expect(messageFrom('HID++ request timed out')).toBe("The device didn't respond in time. Check the connection and try again.");
  });
});
