# CE308 Cloud Computing Project

This repository starts the Rust implementation for the Distributed UAV Telemetry & Fleet Alerting Pipeline.

## Current slice

- Telemetry data model for UAV readings
- Rules engine for critical condition detection
- In-memory deduplication to model alert suppression
- CLI entrypoint for local runs, plus an HTTP server mode for Railway deploys
- MQTT controller bridge plus a drone simulator binary for swarm testing

## Run

```bash
cargo test
cargo run -- < telemetry.json
```

When `PORT` is set, the binary starts an HTTP server instead of reading stdin.

To exercise MQTT locally, start a broker and set the broker details before running the controller or simulator:

```bash
export MQTT_BROKER_HOST=127.0.0.1
export MQTT_BROKER_PORT=1883
export MQTT_SWARM_ID=demo-swarm
export MQTT_TOPIC_PREFIX=ce308

cargo run --bin ce308-cc
cargo run --bin drone_sim
```

Use `MQTT_DEVICE_ID` to run multiple simulator instances with different drone identities.

## Railway deploy

This repo is wired for Railway GitHub autodeploys through [railway.toml](railway.toml) and the root [Dockerfile](Dockerfile).

- `GET /health` returns `200 OK`
- `POST /telemetry` accepts a telemetry reading as JSON
- Railway sets `PORT`, which switches the binary into server mode automatically
- `POST /command/:device_id` publishes an MQTT command to a specific drone when the MQTT broker is configured

## Dashboard

The dashboard is built into the controller service and served from the same Railway deployment. It polls the controller API every few seconds and shows fleet KPIs, drone status, recent alerts, and a command panel.

Local run:

```bash
cd dashboard
npm install
VITE_API_BASE_URL=http://localhost:8080 npm run dev
```

Railway deploy:

- The root [railway.toml](railway.toml) and root [Dockerfile](Dockerfile) deploy the single combined service
- The service exposes both the dashboard UI and the `/api/*` controller endpoints on the same public URL
The dashboard is now served by the same Railway service as the controller. The root [Dockerfile](Dockerfile) builds the React app from [dashboard](dashboard), copies the static bundle into the controller image, and the controller serves the UI from `/`.

## Database compatibility

This project writes plain rows into a `telemetry` table using standard SQL types, so it works with a regular PostgreSQL instance out-of-the-box. On startup the service will create two helpful indexes (`device_id` and `timestamp_ms`) automatically. If TimescaleDB is available the service will also attempt to enable the extension, but failure to enable the extension is non-fatal so the app still runs on standard Postgres.

Example payload:

```json
{
Single-service Railway deploy:

- Keep the repository root as the Railway service root
- Railway uses the root [railway.toml](railway.toml) and [Dockerfile](Dockerfile)
- The controller serves the dashboard HTML at `/` and static assets under `/assets/*`
- Set the controller environment variables only: `PORT`, `DATABASE_URL`, `REDIS_URL`, and MQTT settings
- No separate frontend Railway service is needed
  "gps_lat": 33.6844,
  "gps_lon": 73.0479
}
```
