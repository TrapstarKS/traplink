import Database from 'better-sqlite3';
import { mkdirSync } from 'node:fs';
import { dirname } from 'node:path';
import { readServerConfig } from './config';

export type AppDatabase = Database.Database;

export function openDatabase(path: string): AppDatabase {
  if (path !== ':memory:') mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const db = new Database(path, { timeout: 5000 });
  db.pragma('journal_mode = WAL');
  db.pragma('foreign_keys = ON');
  db.pragma('busy_timeout = 5000');
  db.pragma('synchronous = NORMAL');
  migrateApplicationTables(db);
  return db;
}

export function migrateApplicationTables(db: AppDatabase): void {
  db.exec(`
    CREATE TABLE IF NOT EXISTS app_schema (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS devices (
      id TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES user(id) ON DELETE CASCADE,
      installation_id TEXT NOT NULL,
      name TEXT NOT NULL,
      platform TEXT NOT NULL CHECK (platform IN ('windows', 'macos')),
      app_version TEXT NOT NULL,
      public_key TEXT,
      token_hash TEXT NOT NULL UNIQUE,
      created_at INTEGER NOT NULL,
      last_seen_at INTEGER NOT NULL,
      expires_at INTEGER NOT NULL,
      revoked_at INTEGER,
      UNIQUE (user_id, installation_id)
    );
    CREATE INDEX IF NOT EXISTS devices_owner ON devices(user_id);
    CREATE TABLE IF NOT EXISTS encrypted_backups (
      user_id TEXT PRIMARY KEY REFERENCES user(id) ON DELETE CASCADE,
      payload TEXT NOT NULL,
      sha256 TEXT NOT NULL,
      bytes INTEGER NOT NULL,
      updated_at INTEGER NOT NULL,
      updated_by TEXT NOT NULL REFERENCES devices(id)
    );
    CREATE TABLE IF NOT EXISTS request_limits (
      key TEXT PRIMARY KEY,
      count INTEGER NOT NULL,
      expires_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS request_limits_expiry ON request_limits(expires_at);
    CREATE TABLE IF NOT EXISTS device_approval_claims (
      code_id TEXT PRIMARY KEY,
      session_id TEXT NOT NULL,
      user_id TEXT NOT NULL,
      expires_at INTEGER NOT NULL
    );
    INSERT OR IGNORE INTO app_schema (version, applied_at) VALUES (1, unixepoch() * 1000);
  `);
}

const globalDatabase = globalThis as typeof globalThis & { __traplinkDb?: { path: string; db: AppDatabase } };

export function getDatabase(): AppDatabase {
  const { databasePath } = readServerConfig();
  if (!globalDatabase.__traplinkDb || !globalDatabase.__traplinkDb.db.open) {
    globalDatabase.__traplinkDb = { path: databasePath, db: openDatabase(databasePath) };
  }
  if (globalDatabase.__traplinkDb.path !== databasePath) {
    throw new Error('DATABASE_PATH changed while the server was running. Restart the server.');
  }
  return globalDatabase.__traplinkDb.db;
}
