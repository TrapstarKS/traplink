import { apiRoute, json } from '@/server/http';
import { bearerToken, requireDevice, revokeDevice } from '@/server/devices';

export const runtime = 'nodejs';
export const dynamic = 'force-dynamic';

export async function DELETE(request: Request, { params }: { params: Promise<{ id: string }> }) {
  return apiRoute(request, async db => {
    const { id } = await params;
    return json(revokeDevice(db, requireDevice(db, bearerToken(request)), id));
  });
}
