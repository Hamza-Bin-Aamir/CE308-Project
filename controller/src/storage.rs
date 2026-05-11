use anyhow::Result;
use ce308_core::{mqtt::AckEnvelope, rules, telemetry::TelemetryReading};
use std::sync::Arc;
use tokio_postgres::{Client, NoTls};

pub type Db = Arc<Client>;

#[derive(Debug, Clone)]
pub struct LatestTelemetryRow {
    pub device_id: String,
    pub timestamp_ms: i64,
    pub battery_voltage: Option<f32>,
    pub altitude: Option<f32>,
    pub attitude: Option<f32>,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct AlertRow {
    pub device_id: String,
    pub message_id: String,
    pub timestamp_ms: i64,
    pub rule_kind: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct CommandEventRow {
    pub device_id: String,
    pub message_id: String,
    pub correlation_id: Option<String>,
    pub timestamp_ms: i64,
    pub command_kind: String,
    pub status: String,
    pub detail: Option<String>,
    pub ack_timestamp_ms: Option<i64>,
}

pub async fn init_db(database_url: &str) -> Result<Db> {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("postgres connection error: {e}");
        }
    });

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS telemetry (
                id SERIAL PRIMARY KEY,
                device_id TEXT NOT NULL,
                timestamp_ms BIGINT NOT NULL,
                battery_voltage REAL,
                altitude REAL,
                attitude REAL,
                gps_lat DOUBLE PRECISION,
                gps_lon DOUBLE PRECISION
            )",
            &[],
        )
        .await?;

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS alerts (
                id SERIAL PRIMARY KEY,
                device_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                timestamp_ms BIGINT NOT NULL,
                rule_kind TEXT NOT NULL,
                message TEXT NOT NULL
            )",
            &[],
        )
        .await?;

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS command_events (
                id SERIAL PRIMARY KEY,
                device_id TEXT NOT NULL,
                message_id TEXT NOT NULL UNIQUE,
                correlation_id TEXT,
                timestamp_ms BIGINT NOT NULL,
                command_kind TEXT NOT NULL,
                status TEXT NOT NULL,
                detail TEXT,
                ack_timestamp_ms BIGINT
            )",
            &[],
        )
        .await?;

    let _ = client
        .execute("CREATE INDEX IF NOT EXISTS telemetry_device_idx ON telemetry (device_id)", &[])
        .await;
    let _ = client
        .execute("CREATE INDEX IF NOT EXISTS telemetry_ts_idx ON telemetry (timestamp_ms)", &[])
        .await;
    let _ = client
        .execute("CREATE INDEX IF NOT EXISTS alerts_device_ts_idx ON alerts (device_id, timestamp_ms DESC)", &[])
        .await;
    let _ = client
        .execute("CREATE INDEX IF NOT EXISTS command_events_device_ts_idx ON command_events (device_id, timestamp_ms DESC)", &[])
        .await;
    let _ = client
        .execute("CREATE INDEX IF NOT EXISTS command_events_status_idx ON command_events (status)", &[])
        .await;
    let _ = client
        .execute("CREATE EXTENSION IF NOT EXISTS timescaledb", &[])
        .await;

    Ok(Arc::new(client))
}

pub async fn insert_telemetry(db: &Db, reading: &TelemetryReading) -> Result<()> {
    db.execute(
        "INSERT INTO telemetry (device_id, timestamp_ms, battery_voltage, altitude, attitude, gps_lat, gps_lon) VALUES ($1,$2,$3,$4,$5,$6,$7)",
        &[
            &reading.device_id,
            &(reading.timestamp_ms as i64),
            &reading.battery_voltage_v,
            &reading.altitude_m,
            &reading.attitude_deg,
            &reading.gps_lat,
            &reading.gps_lon,
        ],
    )
    .await?;

    Ok(())
}

pub async fn insert_alert(db: &Db, device_id: &str, message_id: &str, violation: &rules::RuleViolation, timestamp_ms: i64) -> Result<()> {
    db.execute(
        "INSERT INTO alerts (device_id, message_id, timestamp_ms, rule_kind, message) VALUES ($1,$2,$3,$4,$5)",
        &[
            &device_id,
            &message_id,
            &timestamp_ms,
            &format!("{:?}", violation.kind),
            &violation.message,
        ],
    )
    .await?;

    Ok(())
}

pub async fn insert_command_event(
    db: &Db,
    device_id: &str,
    message_id: &str,
    correlation_id: Option<&str>,
    timestamp_ms: i64,
    command_kind: &str,
    status: &str,
    detail: Option<&str>,
) -> Result<()> {
    db.execute(
        "INSERT INTO command_events (device_id, message_id, correlation_id, timestamp_ms, command_kind, status, detail) VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (message_id) DO UPDATE SET correlation_id = EXCLUDED.correlation_id, timestamp_ms = EXCLUDED.timestamp_ms, command_kind = EXCLUDED.command_kind, status = EXCLUDED.status, detail = EXCLUDED.detail",
        &[
            &device_id,
            &message_id,
            &correlation_id,
            &timestamp_ms,
            &command_kind,
            &status,
            &detail,
        ],
    )
    .await?;

    Ok(())
}

pub async fn update_command_ack(db: &Db, ack: &AckEnvelope) -> Result<u64> {
    let rows = db
        .execute(
            "UPDATE command_events SET status = $1, detail = COALESCE($2, detail), ack_timestamp_ms = $3 WHERE message_id = $4 OR correlation_id = $4",
            &[
                &if ack.accepted { "acknowledged" } else { "rejected" },
                &ack.detail,
                &(ack.timestamp_ms as i64),
                &ack.correlation_id,
            ],
        )
        .await?;

    Ok(rows)
}

pub async fn fetch_latest_telemetry(db: &Db) -> Result<Vec<LatestTelemetryRow>> {
    let rows = db
        .query(
            "SELECT DISTINCT ON (device_id) device_id, timestamp_ms, battery_voltage, altitude, attitude, gps_lat, gps_lon
             FROM telemetry
             ORDER BY device_id, timestamp_ms DESC",
            &[],
        )
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| LatestTelemetryRow {
            device_id: row.get(0),
            timestamp_ms: row.get(1),
            battery_voltage: row.get(2),
            altitude: row.get(3),
            attitude: row.get(4),
            gps_lat: row.get(5),
            gps_lon: row.get(6),
        })
        .collect())
}

pub async fn fetch_latest_telemetry_for_device(db: &Db, device_id: &str, limit: i64) -> Result<Vec<LatestTelemetryRow>> {
    let rows = db
        .query(
            "SELECT device_id, timestamp_ms, battery_voltage, altitude, attitude, gps_lat, gps_lon
             FROM telemetry
             WHERE device_id = $1
             ORDER BY timestamp_ms DESC
             LIMIT $2",
            &[&device_id, &limit],
        )
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| LatestTelemetryRow {
            device_id: row.get(0),
            timestamp_ms: row.get(1),
            battery_voltage: row.get(2),
            altitude: row.get(3),
            attitude: row.get(4),
            gps_lat: row.get(5),
            gps_lon: row.get(6),
        })
        .collect())
}

pub async fn fetch_recent_alerts(db: &Db, limit: i64) -> Result<Vec<AlertRow>> {
    let rows = db
        .query(
            "SELECT device_id, message_id, timestamp_ms, rule_kind, message
             FROM alerts
             ORDER BY timestamp_ms DESC
             LIMIT $1",
            &[&limit],
        )
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| AlertRow {
            device_id: row.get(0),
            message_id: row.get(1),
            timestamp_ms: row.get(2),
            rule_kind: row.get(3),
            message: row.get(4),
        })
        .collect())
}

pub async fn fetch_recent_command_events(db: &Db, limit: i64) -> Result<Vec<CommandEventRow>> {
    let rows = db
        .query(
            "SELECT device_id, message_id, correlation_id, timestamp_ms, command_kind, status, detail, ack_timestamp_ms
             FROM command_events
             ORDER BY timestamp_ms DESC
             LIMIT $1",
            &[&limit],
        )
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| CommandEventRow {
            device_id: row.get(0),
            message_id: row.get(1),
            correlation_id: row.get(2),
            timestamp_ms: row.get(3),
            command_kind: row.get(4),
            status: row.get(5),
            detail: row.get(6),
            ack_timestamp_ms: row.get(7),
        })
        .collect())
}

pub async fn fetch_recent_command_events_for_device(db: &Db, device_id: &str, limit: i64) -> Result<Vec<CommandEventRow>> {
    let rows = db
        .query(
            "SELECT device_id, message_id, correlation_id, timestamp_ms, command_kind, status, detail, ack_timestamp_ms
             FROM command_events
             WHERE device_id = $1
             ORDER BY timestamp_ms DESC
             LIMIT $2",
            &[&device_id, &limit],
        )
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| CommandEventRow {
            device_id: row.get(0),
            message_id: row.get(1),
            correlation_id: row.get(2),
            timestamp_ms: row.get(3),
            command_kind: row.get(4),
            status: row.get(5),
            detail: row.get(6),
            ack_timestamp_ms: row.get(7),
        })
        .collect())
}
