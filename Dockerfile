FROM node:20-alpine AS dashboard-builder

WORKDIR /app/dashboard

COPY dashboard/package.json ./
RUN npm install

COPY dashboard/index.html ./
COPY dashboard/vite.config.js ./
COPY dashboard/src ./src

RUN npm run build

FROM rust:1.89-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY core ./core
COPY controller ./controller
COPY simulator ./simulator

RUN cargo build -p ce308-controller --release

FROM debian:bookworm-slim AS runtime

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ce308-controller /app/ce308-controller
COPY --from=dashboard-builder /app/dashboard/dist /app/static

EXPOSE 8080

CMD ["/app/ce308-controller"]