import { apiRoute, json } from '@/server/http';
import { bearerToken, listDevices, requireDevice } from '@/server/devices';

export const runtime = 'nodejs';
export const dynamic = 'force-dynamic';

export async function GET(request: Request) {
  return apiRoute(request, db => json(listDevices(db, requireDevice(db, bearerToken(request)))));
}
