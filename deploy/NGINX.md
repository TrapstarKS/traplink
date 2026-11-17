# Nginx deployment target

Target: `https://traplink.traphub.xyz` on a VPS with an existing Nginx.

Run `docker compose -f compose.nginx.yaml up -d --build` in `deploy`; only `127.0.0.1:3801` is published. Do not start the Caddy compose file on that host.

Install `deploy/nginx.conf` as a dedicated vhost only after `nginx -t` passes. Obtain TLS through the host's existing certificate workflow once DNS resolves. Never overwrite other sites or bind host ports 80/443 from a container.

The product intentionally has no password recovery or SMTP dependency. Keep the no-recovery notice visible at registration.
