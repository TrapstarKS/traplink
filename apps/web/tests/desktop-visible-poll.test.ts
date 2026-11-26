import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { visiblePoll } from '../../desktop/src/lib/visible-poll';

class FakeDocument extends EventTarget {
  hidden = false;
  setHidden(hidden: boolean) { this.hidden = hidden; this.dispatchEvent(new Event('visibilitychange')); }
}

describe('visiblePoll', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('polls only while the document is visible and catches up when shown again', () => {
    const doc = new FakeDocument();
    const load = vi.fn();
    const stop = visiblePoll(load, 1500, doc);
    vi.advanceTimersByTime(4500);
    expect(load).toHaveBeenCalledTimes(3);
    doc.setHidden(true);
    vi.advanceTimersByTime(60_000);
    expect(load).toHaveBeenCalledTimes(3);
    doc.setHidden(false);
    expect(load).toHaveBeenCalledTimes(4);
    stop();
    vi.advanceTimersByTime(10_000);
    doc.setHidden(false);
    expect(load).toHaveBeenCalledTimes(4);
  });
});
