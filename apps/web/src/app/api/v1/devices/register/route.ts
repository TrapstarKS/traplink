import { apiRoute, json, limitRequest, readJson } from '@/server/http';
import { bearerToken, registerDevice, registrationSchema } from '@/server/devices';

export const runtime = 'nodejs';
export const dynamic = 'force-dynamic';

export async function POST(request: Request) {
  return apiRoute(request, async (db, config) => {
    limitRequest(db, request, config, 'enrollment', 10, 60);
    const token = bearerToken(request);
    const body = await readJson(request, registrationSchema);
    return json(registerDevice(db, token, body, config), 201);
  });
}
