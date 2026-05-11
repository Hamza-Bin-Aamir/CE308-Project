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

The dashboard is a separate React + Bootstrap app in [dashboard](dashboard). It polls the controller API every few seconds and shows fleet KPIs, drone status, recent alerts, and a command panel.

Local run:

```bash
cd dashboard
npm install
VITE_API_BASE_URL=http://localhost:8080 npm run dev
```

Railway deploy:

- Create a separate Railway service with root directory set to `dashboard`
- The dashboard service uses [dashboard/railway.toml](dashboard/railway.toml) and [dashboard/Dockerfile](dashboard/Dockerfile)
- Set `VITE_API_BASE_URL` on the dashboard service to the public URL of the controller service
- Keep the controller service configured with `DATABASE_URL`, `REDIS_URL`, and MQTT variables so the dashboard has live data to read

## Database compatibility

This project writes plain rows into a `telemetry` table using standard SQL types, so it works with a regular PostgreSQL instance out-of-the-box. On startup the service will create two helpful indexes (`device_id` and `timestamp_ms`) automatically. If TimescaleDB is available the service will also attempt to enable the extension, but failure to enable the extension is non-fatal so the app still runs on standard Postgres.

Example payload:

```json
{
  "device_id": "uav-01",
  "timestamp_ms": 1715424000000,
  "battery_voltage_v": 10.1,
  "altitude_m": 34.5,
  "attitude_deg": 18.0,
  "gps_lat": 33.6844,
  "gps_lon": 73.0479
}
```
