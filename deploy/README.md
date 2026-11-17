# VPS deployment (authentication)

This deployment publishes only authentication, desktop authorization and private APIs. All management screens live in the desktop app.

## Setup

1. Point the domain's DNS at the VPS and confirm ports 80/443 reach the existing Nginx.
2. Install Docker Engine with Compose.
3. Copy the repository and, in `deploy`, copy `.env.example` to `.env`.
4. Set the domain, the same HTTPS origin in `BETTER_AUTH_URL`, and a random secret of at least 32 characters.
5. On this project's VPS, run `docker compose -f compose.nginx.yaml up -d --build` inside `deploy`. `compose.yaml` (with Caddy) is only for a VPS without an existing proxy.

Migrations run before the app. In Nginx mode the app listens on `127.0.0.1:3801`; enable only the TrapLink vhost and obtain a certificate with Certbot once DNS has propagated. The proxy overwrites `X-Real-IP`, which rate limiting relies on. Never expose the app directly or trust client-supplied headers.

The secret must not be a sample value. Generate it securely, for example with `openssl rand -base64 48`, and keep it out of Git, screenshots and logs.

## Operations

```sh
docker compose ps
docker compose logs --tail=100 auth
docker compose logs --tail=100 migrate
```

Data lives in the `traplink-data` volume. Take consistent backups with the SQLite backup API, or stop the service before copying the database and its WAL files. Never copy only the `.sqlite` file during active writes. Keep the database and the auth secret in protected backups.

This design targets a single app instance. Multiple containers sharing one SQLite file are not a scaling strategy; move to a central database before scaling horizontally.

The service is not a mouse/keyboard relay or public file store. The backup endpoint accepts only bounded, encrypted payloads, and each device can access only its own account's data.

## Before going public

- Verify sign-up, sign-in, device approval and the no-password-recovery notice.
- Replace the placeholder privacy text with operator details, contact, retention and deletion process.
- Set up monitoring, backups and an update routine.
- Validate VPS limits and capacity under load; do not run multiple SQLite instances.
- Run the user-isolation and revocation tests before every release.
