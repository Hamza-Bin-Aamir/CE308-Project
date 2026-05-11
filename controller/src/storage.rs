use anyhow::Result;
use tokio_postgres::{NoTls, Client};
use ce308_core::telemetry::TelemetryReading;
use std::sync::Arc;

pub type Db = Arc<Client>;

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

    let _ = client
        .execute("CREATE INDEX IF NOT EXISTS telemetry_device_idx ON telemetry (device_id)", &[])
        .await;
    let _ = client
        .execute("CREATE INDEX IF NOT EXISTS telemetry_ts_idx ON telemetry (timestamp_ms)", &[])
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
