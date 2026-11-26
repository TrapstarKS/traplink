import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { createFlags, isPrerelease, publishFlags } from '../../../scripts/release/release-channel.mjs';

describe('release channel', () => {
  it('never publishes a prerelease tag as Latest', () => {
    for (const tag of ['v0.2.0-rc.1', 'v1.2.3-beta.1', 'v1.0.0-alpha']) {
      expect(isPrerelease(tag)).toBe(true);
      expect(publishFlags(tag)).toEqual(['--prerelease', '--latest=false']);
      expect(publishFlags(tag)).not.toContain('--latest');
      expect(createFlags(tag)).toEqual(['--prerelease']);
    }
  });

  it('keeps publishing stable tags as Latest', () => {
    for (const tag of ['v0.1.10', 'v1.2.3']) {
      expect(isPrerelease(tag)).toBe(false);
      expect(publishFlags(tag)).toEqual(['--latest']);
      expect(createFlags(tag)).toEqual([]);
    }
  });

  it('is what the release script uses to publish and create releases', () => {
    const script = readFileSync(new URL('../../../scripts/release/release.mjs', import.meta.url), 'utf8');
    expect(script).toContain('...publishFlags(releaseTag)');
    expect(script).toContain('...createFlags(releaseTag)');
    expect(script).not.toMatch(/"--draft=false", "--latest"\]/);
  });
});
