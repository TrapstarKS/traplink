import { describe, expect, it } from 'vitest';
import { inviteBaseline, inviteState } from '../../desktop/src/lib/pairing';

const invite = { code: 'ABC-123', expiresAt: '2026-01-01T00:05:00.000Z' };
const before = Date.parse('2026-01-01T00:01:00.000Z');
const after = Date.parse('2026-01-01T00:06:00.000Z');
const desk = { id: 'desk', name: 'Desk PC', connected: true };
const laptop = { id: 'laptop', name: 'Laptop', connected: true };

describe('inviteState', () => {
  it('is available while the core still holds the invite', () => {
    expect(inviteState(invite, 'ABC-123', [desk], inviteBaseline([desk]), before)).toEqual({ kind: 'available' });
  });

  it('reports the new computer once the invite is consumed by a pairing', () => {
    expect(inviteState(invite, null, [desk, laptop], inviteBaseline([desk]), before)).toEqual({ kind: 'paired', peer: { id: 'laptop', name: 'Laptop' } });
  });

  it('reports a known computer that paired again and connected', () => {
    const offline = { ...laptop, connected: false };
    expect(inviteState(invite, null, [desk, laptop], inviteBaseline([desk, offline]), before)).toEqual({ kind: 'paired', peer: { id: 'laptop', name: 'Laptop' } });
  });

  it('stays closed when the invite is gone without a new connection', () => {
    expect(inviteState(invite, null, [desk], inviteBaseline([desk]), before)).toEqual({ kind: 'closed' });
  });

  it('expires after the deadline', () => {
    expect(inviteState(invite, null, [desk], inviteBaseline([desk]), after)).toEqual({ kind: 'expired' });
    expect(inviteState(invite, 'ABC-123', [desk], inviteBaseline([desk]), after)).toEqual({ kind: 'expired' });
  });
});
