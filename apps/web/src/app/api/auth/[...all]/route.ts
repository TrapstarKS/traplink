import { getAuth } from '@/lib/auth';
import { readServerConfig } from '@/server/config';
import { getDatabase } from '@/server/database';
import { json } from '@/server/http';
import { handleAuthRequest } from '@/server/auth-handler';

export const runtime = 'nodejs';
export const dynamic = 'force-dynamic';

async function handler(request: Request) {
  try {
    const config = readServerConfig();
    return handleAuthRequest(request, getDatabase(), config, getAuth());
  } catch {
    return json({ error: 'service_unavailable', message: 'Authentication is temporarily unavailable.' }, 503);
  }
}

export { handler as GET, handler as POST };
