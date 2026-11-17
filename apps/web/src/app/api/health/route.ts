import { json } from '@/server/http';
import { getDatabase } from '@/server/database';
import { readServerConfig } from '@/server/config';

export const runtime = 'nodejs';
export const dynamic = 'force-dynamic';

export async function GET() {
  try {
    readServerConfig();
    getDatabase().prepare('SELECT id FROM user LIMIT 1').get();
    return json({ status: 'ok' });
  } catch {
    return json({ status: 'unavailable' }, 503);
  }
}
