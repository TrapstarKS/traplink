// Pure helpers for release channel decisions. No side effects on import, so tests can load this file.

/** True for semver prerelease tags such as v1.2.3-rc.1 or v1.2.3-beta.2. */
export function isPrerelease(releaseTag) {
  return /^v\d+\.\d+\.\d+-[0-9A-Za-z.-]+$/.test(releaseTag);
}

/** Extra flags for `gh release create` (draft) so prereleases are marked from the start. */
export function createFlags(releaseTag) {
  return isPrerelease(releaseTag) ? ["--prerelease"] : [];
}

/**
 * Flags for the final `gh release edit`. The desktop updater reads
 * /releases/latest/download/latest.json, so only stable tags may become Latest.
 */
export function publishFlags(releaseTag) {
  return isPrerelease(releaseTag) ? ["--prerelease", "--latest=false"] : ["--latest"];
}
