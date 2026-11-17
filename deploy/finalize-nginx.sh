#!/usr/bin/env bash
set -euo pipefail

domain=traplink.traphub.xyz
root=$(cd "$(dirname "$0")/.." && pwd)
site=/etc/nginx/sites-available/traplink
enabled=/etc/nginx/sites-enabled/traplink

if ! getent ahostsv4 "$domain" >/dev/null; then
  echo "DNS pending: $domain does not resolve."
  exit 2
fi
test "$(curl --fail --silent http://127.0.0.1:3801/api/health)" = '{"status":"ok"}'
nginx -t
backup=$(mktemp -d /tmp/traplink-nginx.XXXXXX)
had_site=false
had_enabled=false
test ! -e "$site" || { cp -p "$site" "$backup/site"; had_site=true; }
if test -e "$enabled" || test -L "$enabled"; then
  cp -Pp "$enabled" "$backup/enabled"
  had_enabled=true
fi
committed=false
cleanup() {
  code=$?
  if test "$committed" = false; then
    if test "$had_site" = true; then cp -p "$backup/site" "$site"; else rm -f "$site"; fi
    rm -f "$enabled"
    if test "$had_enabled" = true; then cp -Pp "$backup/enabled" "$enabled"; fi
    if nginx -t; then systemctl reload nginx; fi
    echo "TrapLink vhost restored after incomplete deployment. Backup: $backup" >&2
  fi
  test -z "${challenge:-}" || rm -f "/var/www/traplink-acme/.well-known/acme-challenge/$challenge"
  exit "$code"
}
trap cleanup EXIT
mkdir -p /var/www/traplink-acme
install -m 0644 "$root/deploy/nginx.conf" "$site"
if ! test -e "$enabled" && ! test -L "$enabled"; then ln -s "$site" "$enabled"; fi
if ! nginx -t; then
  echo "Nginx validation failed. Existing worker configuration was not reloaded."
  exit 1
fi
systemctl reload nginx
mkdir -p /var/www/traplink-acme/.well-known/acme-challenge
challenge="traplink-$(openssl rand -hex 16)"
printf '%s' "$challenge" > "/var/www/traplink-acme/.well-known/acme-challenge/$challenge"
actual=$(curl --fail --silent --show-error --retry 6 --retry-all-errors --retry-delay 2 --max-time 10 "http://$domain/.well-known/acme-challenge/$challenge")
if test "$actual" != "$challenge"; then
  echo "Public DNS/proxy does not reach the TrapLink challenge on this VPS." >&2
  exit 2
fi
certbot certonly --webroot -w /var/www/traplink-acme -d "$domain" --non-interactive --agree-tos --register-unsafely-without-email \
  --deploy-hook '/usr/sbin/nginx -t && /usr/bin/systemctl reload nginx'

cat > "$site" <<'NGINX'
server {
    listen 80;
    listen [::]:80;
    server_name traplink.traphub.xyz;
    location /.well-known/acme-challenge/ { root /var/www/traplink-acme; }
    location / { return 301 https://$host$request_uri; }
}
server {
    listen 443 ssl;
    listen [::]:443 ssl;
    server_name traplink.traphub.xyz;
    ssl_certificate /etc/letsencrypt/live/traplink.traphub.xyz/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/traplink.traphub.xyz/privkey.pem;
    ssl_protocols TLSv1.2 TLSv1.3;
    client_max_body_size 12m;
    add_header Strict-Transport-Security "max-age=31536000" always;
    add_header X-Content-Type-Options nosniff always;
    add_header X-Frame-Options DENY always;
    add_header Referrer-Policy same-origin always;
    location / {
        proxy_pass http://127.0.0.1:3801;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 30s;
        proxy_connect_timeout 5s;
    }
}
NGINX
nginx -t
systemctl reload nginx
healthy=false
for _ in 1 2 3 4 5; do
  result=$(curl --fail --silent --show-error --max-time 10 "https://$domain/api/health?traplink-deploy=$challenge") || result=''
  if test "$result" = '{"status":"ok"}'; then healthy=true; break; fi
  sleep 2
done
if test "$healthy" != true; then
  echo 'The public HTTPS response did not match the TrapLink health endpoint.' >&2
  exit 1
fi
committed=true
echo "TrapLink HTTPS ready: https://$domain"
