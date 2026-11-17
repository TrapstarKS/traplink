#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { appendFileSync, copyFileSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const tauriDir = join(root, "apps", "desktop", "src-tauri");
const ALL_PLATFORMS = {
  windows: { key: "windows-x86_64" },
  macos: { key: "darwin-aarch64" },
};
const requested = (process.env.RELEASE_PLATFORMS || "windows,macos").split(",").map(name => name.trim()).filter(Boolean);
for (const name of requested) if (!ALL_PLATFORMS[name]) { console.error(`error: unknown platform "${name}" in RELEASE_PLATFORMS`); process.exit(1); }
const PLATFORMS = Object.fromEntries(requested.map(name => [name, ALL_PLATFORMS[name]]));

function fail(message) { console.error(`error: ${message}`); process.exit(1); }
const TRANSIENT = /error connecting|connection (reset|refused)|timed? ?out|timeout|EOF|TLS handshake|no such host|temporary failure|HTTP (429|5\d\d)|502|503|504/i;
function pause(ms) { Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms); }
function gh(args, { allowFailure = false, capture = true } = {}) {
  const retryable = !(args[0] === "release" && args[1] === "create");
  let result;
  for (let attempt = 1; attempt <= 6; attempt++) {
    result = spawnSync("gh", args, { encoding: "utf8", stdio: ["ignore", capture ? "pipe" : "inherit", "pipe"], shell: false });
    if (result.error) fail(`could not run gh (${result.error.message}). Install GitHub CLI on this runner.`);
    if (!capture && result.stderr) process.stderr.write(result.stderr);
    if (result.status === 0 || !retryable || !TRANSIENT.test(result.stderr || "") || attempt === 6) break;
    const wait = Math.min(60000, 2000 * 2 ** (attempt - 1));
    console.error(`gh ${args[0]} ${args[1] ?? ""} hit a network error, retrying in ${wait / 1000}s (attempt ${attempt + 1}/6)`);
    pause(wait);
  }
  if (result.status !== 0 && !allowFailure) fail(`gh ${args[0]} ${args[1] ?? ""} failed: ${(result.stderr || "").trim()}`);
  return result;
}
function repository() {
  const repo = process.env.GITHUB_REPOSITORY;
  if (!repo || !/^[\w.-]+\/[\w.-]+$/.test(repo)) fail("GITHUB_REPOSITORY must be set to owner/repo.");
  return repo;
}
function tag() {
  const value = process.env.RELEASE_TAG ?? "";
  if (!/^v\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(value)) fail(`RELEASE_TAG "${value}" must look like v1.2.3.`);
  return value;
}
function versions() {
  const conf = JSON.parse(readFileSync(join(tauriDir, "tauri.conf.json"), "utf8")).version;
  const pkg = JSON.parse(readFileSync(join(root, "apps", "desktop", "package.json"), "utf8")).version;
  const cargo = /^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m.exec(readFileSync(join(tauriDir, "Cargo.toml"), "utf8"))?.[1];
  return { conf, pkg, cargo };
}
function output(name, value) {
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${name}=${value}\n`);
}
function releaseInfo(releaseTag) {
  const view = gh(["release", "view", releaseTag, "--repo", repository(), "--json", "isDraft,body,assets"], { allowFailure: true });
  return view.status === 0 ? JSON.parse(view.stdout) : null;
}
function targetDir() {
  if (process.env.CARGO_TARGET_DIR) return resolve(process.env.CARGO_TARGET_DIR);
  return join(tauriDir, "target");
}
function find(dir, predicate) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir).filter(predicate).map(name => join(dir, name));
}
function single(files, description) {
  if (files.length !== 1) fail(`expected exactly one ${description}, found ${files.length}: ${files.join(", ") || "none"}`);
  return files[0];
}

function prepare() {
  const releaseTag = tag();
  const version = releaseTag.slice(1);
  const found = versions();
  for (const [file, value] of Object.entries(found)) {
    if (value !== version) fail(`${file} version is "${value}" but the tag is ${releaseTag}. Set the same version in tauri.conf.json, Cargo.toml and apps/desktop/package.json.`);
  }
  const existing = releaseInfo(releaseTag);
  if (existing && !existing.isDraft) fail(`release ${releaseTag} is already published. Create a new version instead of replacing published files.`);
  if (!existing) {
    gh(["release", "create", releaseTag, "--repo", repository(), "--draft", "--verify-tag", "--generate-notes", "--title", `TrapLink ${version}`]);
    console.log(`Created draft release ${releaseTag}.`);
  } else {
    console.log(`Reusing draft release ${releaseTag}.`);
  }
  output("tag", releaseTag);
  output("version", version);
}

function upload(platform) {
  const config = PLATFORMS[platform];
  if (!config) fail(`unknown platform "${platform}". Use ${Object.keys(PLATFORMS).join(" or ")}.`);
  const releaseTag = tag();
  const version = releaseTag.slice(1);
  const repo = repository();
  const bundle = join(targetDir(), "release", "bundle");
  const staging = mkdtempSync(join(tmpdir(), "traplink-release-"));
  const files = [];
  let updater;
  let signature;

  if (platform === "windows") {
    const nsis = join(bundle, "nsis");
    const installer = single(find(nsis, name => name.endsWith(`_${version}_x64-setup.exe`)), `NSIS installer for ${version} in ${nsis}`);
    const sig = `${installer}.sig`;
    if (!existsSync(sig)) fail(`missing updater signature ${sig}. Is TAURI_SIGNING_PRIVATE_KEY set?`);
    const name = `TrapLink_${version}_x64-setup.exe`;
    copyFileSync(installer, join(staging, name));
    copyFileSync(sig, join(staging, `${name}.sig`));
    files.push(name, `${name}.sig`);
    updater = name;
    signature = readFileSync(sig, "utf8").trim();
  } else {
    const dmg = single(find(join(bundle, "dmg"), name => name.endsWith(`_${version}_aarch64.dmg`)), `DMG for ${version}`);
    const archive = single(find(join(bundle, "macos"), name => name.endsWith(".app.tar.gz")), "macOS updater archive (.app.tar.gz)");
    const sig = `${archive}.sig`;
    if (!existsSync(sig)) fail(`missing updater signature ${sig}. Is TAURI_SIGNING_PRIVATE_KEY set?`);
    const dmgName = `TrapLink_${version}_aarch64.dmg`;
    const archiveName = `TrapLink_${version}_aarch64.app.tar.gz`;
    copyFileSync(dmg, join(staging, dmgName));
    copyFileSync(archive, join(staging, archiveName));
    copyFileSync(sig, join(staging, `${archiveName}.sig`));
    files.push(dmgName, archiveName, `${archiveName}.sig`);
    updater = archiveName;
    signature = readFileSync(sig, "utf8").trim();
  }

  const fragment = `updater-${config.key}.json`;
  writeFileSync(join(staging, fragment), JSON.stringify({
    [config.key]: { signature, url: `https://github.com/${repo}/releases/download/${releaseTag}/${updater}` },
  }, null, 2));
  files.push(fragment);

  gh(["release", "upload", releaseTag, "--repo", repo, "--clobber", ...files.map(name => join(staging, name))], { capture: false });
  rmSync(staging, { recursive: true, force: true });
  console.log(`Uploaded ${files.join(", ")} to ${releaseTag}.`);
}

function finalize() {
  const releaseTag = tag();
  const repo = repository();
  const info = releaseInfo(releaseTag);
  if (!info) fail(`release ${releaseTag} does not exist.`);
  const work = mkdtempSync(join(tmpdir(), "traplink-latest-"));
  gh(["release", "download", releaseTag, "--repo", repo, "--pattern", "updater-*.json", "--dir", work, "--clobber"]);
  const platforms = {};
  for (const file of find(work, name => /^updater-.+\.json$/.test(name))) Object.assign(platforms, JSON.parse(readFileSync(file, "utf8")));
  for (const { key } of Object.values(PLATFORMS)) if (!platforms[key]) fail(`missing updater entry for ${key}. Wait for every platform job to upload before publishing.`);

  const latest = { version: releaseTag.slice(1), notes: (info.body || "").trim(), pub_date: new Date().toISOString(), platforms };
  const latestPath = join(work, "latest.json");
  writeFileSync(latestPath, `${JSON.stringify(latest, null, 2)}\n`);
  gh(["release", "upload", releaseTag, "--repo", repo, "--clobber", latestPath], { capture: false });
  for (const { key } of Object.values(PLATFORMS)) gh(["release", "delete-asset", releaseTag, `updater-${key}.json`, "--repo", repo, "--yes"], { allowFailure: true });
  gh(["release", "edit", releaseTag, "--repo", repo, "--draft=false", "--latest"], { capture: false });
  rmSync(work, { recursive: true, force: true });
  console.log(`Published ${releaseTag} with latest.json for ${Object.keys(platforms).join(", ")}.`);
}

function prune(args) {
  const keepIndex = args.indexOf("--keep");
  const keep = keepIndex >= 0 ? Number(args[keepIndex + 1]) : 3;
  if (!Number.isInteger(keep) || keep < 1) fail("--keep must be a positive integer.");
  const apply = args.includes("--yes");
  const repo = repository();
  const list = JSON.parse(gh(["release", "list", "--repo", repo, "--limit", "200", "--json", "tagName,isDraft,isLatest,publishedAt"]).stdout);
  const published = list.filter(release => !release.isDraft).sort((a, b) => Date.parse(b.publishedAt) - Date.parse(a.publishedAt));
  const remove = published.slice(keep).filter(release => !release.isLatest);
  if (!remove.length) { console.log(`Nothing to prune. ${published.length} published release(s), keeping ${keep}.`); return; }
  for (const release of remove) {
    if (!apply) { console.log(`[dry run] would delete ${release.tagName} and its tag`); continue; }
    gh(["release", "delete", release.tagName, "--repo", repo, "--cleanup-tag", "--yes"], { capture: false });
    console.log(`Deleted ${release.tagName}.`);
  }
  if (!apply) console.log("Dry run only. Pass --yes to delete.");
}

const [command, ...rest] = process.argv.slice(2);
if (command === "prepare") prepare();
else if (command === "upload") upload(rest[0]);
else if (command === "finalize") finalize();
else if (command === "prune") prune(rest);
else fail("usage: release.mjs prepare | upload <windows|macos> | finalize | prune [--keep 3] [--yes]");
