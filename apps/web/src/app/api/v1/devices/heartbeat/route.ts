import { apiRoute, json } from '@/server/http';
import { bearerToken, heartbeat, requireDevice } from '@/server/devices';

export const runtime = 'nodejs';
export const dynamic = 'force-dynamic';

export async function POST(request: Request) {
  return apiRoute(request, db => json(heartbeat(db, requireDevice(db, bearerToken(request)))));
}
