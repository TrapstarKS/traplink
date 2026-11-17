import { readFileSync, existsSync } from 'node:fs';
import { parseEnv } from 'node:util';
import { resolve } from 'node:path';
import { getMigrations } from 'better-auth/db/migration';
import { createAuth } from '../src/lib/auth';
import { openDatabase, migrateApplicationTables } from '../src/server/database';
import { readServerConfig } from '../src/server/config';

for (const filename of ['.env.local', '.env']) {
  const path = resolve(process.cwd(), filename);
  if (!existsSync(path)) continue;
  for (const [key, value] of Object.entries(parseEnv(readFileSync(path, 'utf8')))) {
    if (process.env[key] === undefined) process.env[key] = value;
  }
}

async function main() {
  const config = readServerConfig();
  const db = openDatabase(config.databasePath);
  try {
    const instance = createAuth(db, config);
    const migrations = await getMigrations(instance.options);
    if (migrations.schemaProblems.length) throw new Error('The authentication schema requires manual migration review.');
    await migrations.runMigrations();
    migrateApplicationTables(db);
    console.info('TrapLink database migrations completed.');
  } finally {
    db.close();
  }
}
main().catch(() => { console.error('Migration failed. Check database access and configuration; secrets were not logged.'); process.exitCode = 1; });
