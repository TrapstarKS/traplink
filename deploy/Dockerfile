FROM node:22-bookworm-slim AS builder
WORKDIR /app
RUN corepack enable && corepack prepare pnpm@10.10.0 --activate
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps/web/package.json ./apps/web/package.json
COPY packages/shared/package.json ./packages/shared/package.json
RUN pnpm install --frozen-lockfile --filter @traplink/web...
COPY apps/web ./apps/web
COPY packages/shared ./packages/shared
RUN pnpm --filter @traplink/web build

FROM node:22-bookworm-slim AS runtime
ENV NODE_ENV=production PORT=3100 HOSTNAME=0.0.0.0
WORKDIR /app
RUN groupadd --gid 10001 traplink && useradd --uid 10001 --gid traplink --no-create-home traplink \
    && mkdir -p /data && chown traplink:traplink /data
COPY --from=builder --chown=traplink:traplink /app/apps/web/.next/standalone ./
COPY --from=builder --chown=traplink:traplink /app/apps/web/.next/static ./apps/web/.next/static
COPY --from=builder --chown=traplink:traplink /app/apps/web/public ./apps/web/public
USER traplink
EXPOSE 3100
CMD ["node", "apps/web/server.js"]

FROM builder AS migrate
ENV NODE_ENV=production
RUN groupadd --gid 10001 traplink && useradd --uid 10001 --gid traplink --no-create-home traplink \
    && mkdir -p /data && chown traplink:traplink /data
WORKDIR /app/apps/web
USER traplink
CMD ["node", "--import", "tsx", "scripts/migrate.ts"]
