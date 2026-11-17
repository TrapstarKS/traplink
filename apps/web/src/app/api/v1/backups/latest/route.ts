import { apiRoute, json, limitRequest, readJson } from '@/server/http';
import { bearerToken, requireDevice } from '@/server/devices';
import { backupSchema, deleteBackup, loadBackup, saveBackup } from '@/server/backups';

export const runtime = 'nodejs';
export const dynamic = 'force-dynamic';

export async function GET(request: Request) {
  return apiRoute(request, db => json(loadBackup(db, requireDevice(db, bearerToken(request)))));
}

export async function PUT(request: Request) {
  return apiRoute(request, async (db, config) => {
    limitRequest(db, request, config, 'backup-upload', 10, 60);
    const token = bearerToken(request);
    requireDevice(db, token);
    const envelope = await readJson(request, backupSchema, config.maxBackupBytes);
    return json(saveBackup(db, requireDevice(db, token), envelope, config.maxBackupBytes));
  });
}

export async function DELETE(request: Request) {
  return apiRoute(request, db => json(deleteBackup(db, requireDevice(db, bearerToken(request)))));
}
