import { z } from 'zod';
import { readServerConfig, type ServerConfig } from './config';
import { getDatabase, type AppDatabase } from './database';
import { consumeLimit, requestAddress } from './rate-limit';

export class ApiError extends Error {
  constructor(readonly status: number, readonly code: string, message: string) { super(message); }
}

export function json(data: unknown, status = 200, extraHeaders?: HeadersInit): Response {
  const headers = new Headers(extraHeaders);
  headers.set('Cache-Control', 'no-store');
  headers.set('Pragma', 'no-cache');
  headers.set('X-Content-Type-Options', 'nosniff');
  return Response.json(data, { status, headers });
}

export async function readJson<T>(request: Request, schema: z.ZodType<T>, maxBytes = 16_384): Promise<T> {
  if (request.headers.get('content-type')?.split(';')[0].trim().toLowerCase() !== 'application/json') {
    throw new ApiError(415, 'unsupported_media_type', 'Send a JSON request.');
  }
  const declared = Number(request.headers.get('content-length') ?? '0');
  if (!Number.isFinite(declared) || declared > maxBytes) throw new ApiError(413, 'payload_too_large', 'The request is too large.');
  const reader = request.body?.getReader();
  if (!reader) throw new ApiError(400, 'invalid_request', 'A JSON body is required.');
  const chunks: Uint8Array[] = [];
  let total = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > maxBytes) {
      await reader.cancel();
      throw new ApiError(413, 'payload_too_large', 'The request is too large.');
    }
    chunks.push(value);
  }
  let input: unknown;
  try { input = JSON.parse(Buffer.concat(chunks).toString('utf8')); }
  catch { throw new ApiError(400, 'invalid_request', 'The request is not valid JSON.'); }
  const parsed = schema.safeParse(input);
  if (!parsed.success) throw new ApiError(400, 'invalid_request', 'One or more request fields are invalid.');
  return parsed.data;
}

export function assertRequestOrigin(request: Request, config: ServerConfig): void {
  const origin = request.headers.get('origin');
  if (origin && origin !== config.baseURL) throw new ApiError(403, 'untrusted_origin', 'This request origin is not allowed.');
}

export function limitRequest(db: AppDatabase, request: Request, config: ServerConfig, bucket: string, max: number, seconds: number): void {
  const result = consumeLimit(db, `${bucket}:${requestAddress(request, config)}`, max, seconds);
  if (!result.allowed) throw new ApiError(429, 'rate_limited', `Too many requests. Retry in ${result.retryAfter} seconds.`);
}

export async function apiRoute(request: Request, handler: (db: AppDatabase, config: ServerConfig) => Response | Promise<Response>): Promise<Response> {
  try {
    const config = readServerConfig();
    const db = getDatabase();
    assertRequestOrigin(request, config);
    limitRequest(db, request, config, 'api', 180, 60);
    return await handler(db, config);
  } catch (error) {
    if (error instanceof ApiError) return json({ error: error.code, message: error.message }, error.status);
    return json({ error: 'service_unavailable', message: 'The service is temporarily unavailable.' }, 503);
  }
}
