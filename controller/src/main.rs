use axum::{extract::{Path, State}, http::StatusCode, response::Html, routing::{get, post}, Json, Router};
use ce308_core::{dedupe::AlertDedupe, pipeline, telemetry::TelemetryReading};
use ce308_core::mqtt;
use redis_adapter::{create_pool_from_url, set_alert_key, RedisPool};
use serde::Serialize;
use std::{collections::HashMap, sync::{Arc, Mutex}, time::{SystemTime, UNIX_EPOCH}};
use storage::{
    fetch_latest_telemetry, fetch_latest_telemetry_for_device, fetch_recent_alerts,
    fetch_recent_command_events, fetch_recent_command_events_for_device, init_db,
    insert_alert, insert_command_event, insert_telemetry, update_command_ack, AlertRow,
    CommandEventRow, Db, LatestTelemetryRow,
};
use tower_http::{cors::{Any, CorsLayer}, services::ServeDir};

mod storage;
mod redis_adapter;

#[tokio::main]
async fn main() {
    if let Some(port) = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
    {
        run_server(port).await;
        return;
    }

    eprintln!("controller must be run with PORT for server mode");
}

struct AppState {
    dedupe: Arc<Mutex<AlertDedupe>>,
    db: Option<Db>,
    redis: Option<RedisPool>,
    mqtt: Arc<Mutex<Option<mqtt::MqttHandle>>>,
}

async fn run_server(port: u16) {
    // optional DB and Redis initialization from env
    let db = std::env::var("DATABASE_URL").ok();
    let redis_url = std::env::var("REDIS_URL").ok();

    let db_pool = if let Some(url) = db.as_deref() {
        match init_db(url).await {
            Ok(pool) => Some(pool),
            Err(e) => {
                eprintln!("failed to init db: {e}");
                None
            }
        }
    } else {
        None
    };

    let redis_pool = if let Some(url) = redis_url.as_deref() {
        match create_pool_from_url(url) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("failed to init redis: {e}");
                None
            }
        }
    } else {
        None
    };

    let shared = Arc::new(AppState {
        dedupe: Arc::new(Mutex::new(AlertDedupe::new(60_000))),
        db: db_pool,
        redis: redis_pool,
        mqtt: Arc::new(Mutex::new(None)),
    });

    if let Some(config) = mqtt::MqttConfig::from_env("ce308-controller") {
        let telemetry_state = shared.clone();
        let ack_state = shared.clone();
        match mqtt::connect_controller_with_ack(
            config,
            move |envelope| {
                let telemetry_state = telemetry_state.clone();
                async move { process_reading_with_state(telemetry_state, envelope.reading).await }
            },
            move |ack| {
                let ack_state = ack_state.clone();
                async move {
                    if let Some(db) = &ack_state.db {
                        if let Err(error) = update_command_ack(db, &ack).await {
                            eprintln!("failed to persist command ack: {error}");
                        }
                    }

                    Ok(())
                }
            },
        )
        .await
        {
            Ok(handle) => {
                if let Ok(mut guard) = shared.mqtt.lock() {
                    *guard = Some(handle);
                }
            }
            Err(error) => eprintln!("failed to initialize mqtt controller: {error}"),
        }
    }

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/telemetry", post(ingest_telemetry))
        .route("/command/:device_id", post(send_command))
        .route("/api/summary", get(api_summary))
        .route("/api/drones", get(api_drones))
        .route("/api/drones/:device_id", get(api_drone_detail))
        .route("/api/alerts/recent", get(api_recent_alerts))
        .route("/api/commands/recent", get(api_recent_commands))
        .route("/api/command/:device_id", post(send_command))
        .nest_service("/assets", ServeDir::new("/app/static/assets").precompressed_gzip())
        .route("/", get(index_page))
        .route("/*path", get(spa_fallback))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .with_state(shared.clone());

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("binding TCP listener");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("running HTTP server");
}

async fn health_check() -> StatusCode {
    StatusCode::OK
}

async fn index_page() -> Result<Html<String>, (StatusCode, Json<serde_json::Value>)> {
    serve_index().await
}

async fn spa_fallback(Path(path): Path<String>) -> Result<Html<String>, (StatusCode, Json<serde_json::Value>)> {
    if path.starts_with("api/") {
        return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "not_found"}))));
    }

    serve_index().await
}

async fn serve_index() -> Result<Html<String>, (StatusCode, Json<serde_json::Value>)> {
    let contents = tokio::fs::read_to_string("/app/static/index.html").await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("failed to read dashboard: {error}")})),
        )
    })?;

    Ok(Html(contents))
}

async fn ingest_telemetry(
    State(state): State<Arc<AppState>>,
    Json(reading): Json<TelemetryReading>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    match process_reading_with_state(state, reading).await {
        Ok(outcome) => {
            let body = serde_json::to_value(&outcome).map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("failed to serialize outcome: {error}")})),
                )
            })?;

            Ok(Json(body))
        }
        Err(error) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error.to_string()})),
        )),
    }
}

async fn send_command(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
    Json(command): Json<mqtt::SwarmCommand>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let handle = state
        .mqtt
        .lock()
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error":"mqtt_lock_failed"}))))?
        .clone()
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"mqtt_not_configured"})),
        ))?;

    let envelope = handle.publish_command(&device_id, command).await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": format!("failed to publish command: {error}")})),
        )
    })?;

    if let Some(db) = &state.db {
        let db = db.clone();
        let device_id = device_id.clone();
        let command_kind = envelope.command.kind_name().to_string();
        let message_id = envelope.message_id.clone();
        let timestamp_ms = envelope.timestamp_ms as i64;
        tokio::spawn(async move {
            if let Err(error) = insert_command_event(
                &db,
                &device_id,
                &message_id,
                envelope.correlation_id.as_deref(),
                timestamp_ms,
                &command_kind,
                "queued",
                Some("published to mqtt"),
            )
            .await
            {
                eprintln!("failed to persist command event: {error}");
            }
        });
    }

    Ok(Json(serde_json::json!({
        "status": "queued",
        "device_id": device_id,
        "message_id": envelope.message_id,
        "topic": handle.command_topic(&envelope.device_id),
    })))
}

pub(crate) async fn process_reading_with_state(
    state: Arc<AppState>,
    reading: TelemetryReading,
) -> anyhow::Result<pipeline::AlertOutcome> {
    let outcome = {
        let mut dedupe = state
            .dedupe
            .lock()
            .map_err(|_| anyhow::anyhow!("dedupe lock poisoned"))?;

        pipeline::process_reading(&reading, &mut dedupe)
    };

    if let Some(db) = &state.db {
        let db = db.clone();
        let reading_clone = reading.clone();
        let alerts = outcome.emitted_alerts.clone();
        tokio::spawn(async move {
            if let Err(e) = insert_telemetry(&db, &reading_clone).await {
                eprintln!("failed to persist telemetry: {e}");
            }

            let message_id_base = reading_clone.timestamp_ms as i64;
            for (index, violation) in alerts.iter().enumerate() {
                let message_id = format!("alert-{}-{}-{}", reading_clone.device_id, message_id_base, index);
                if let Err(error) = insert_alert(&db, &reading_clone.device_id, &message_id, violation, message_id_base).await {
                    eprintln!("failed to persist alert: {error}");
                }
            }
        });
    }

    if let Some(redis_pool) = &state.redis {
        if !outcome.emitted_alerts.is_empty() {
            let pool = redis_pool.clone();
            let device = reading.device_id.clone();
            let alerts = outcome.emitted_alerts.clone();
            tokio::spawn(async move {
                for violation in alerts {
                    let key = format!("alert:{}:{}", device, serde_json::to_string(&violation.kind).unwrap_or_default());
                    if let Err(e) = set_alert_key(&pool, &key, 60).await {
                        eprintln!("failed to set redis alert key: {e}");
                    }
                }
            });
        }
    }

    Ok(outcome)
}

async fn api_summary(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let db = required_db(&state)?;
    let telemetry = fetch_latest_telemetry(&db).await.map_err(internal_error)?;
    let alerts = fetch_recent_alerts(&db, 100).await.map_err(internal_error)?;
    let commands = fetch_recent_command_events(&db, 100).await.map_err(internal_error)?;
    Ok(Json(build_summary(&telemetry, &alerts, &commands)))
}

async fn api_drones(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let db = required_db(&state)?;
    let telemetry = fetch_latest_telemetry(&db).await.map_err(internal_error)?;
    let commands = fetch_recent_command_events(&db, 200).await.map_err(internal_error)?;
    Ok(Json(serde_json::json!(merge_drones(&telemetry, &commands))))
}

async fn api_drone_detail(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let db = required_db(&state)?;
    let telemetry = fetch_latest_telemetry_for_device(&db, &device_id, 50).await.map_err(internal_error)?;
    let commands = fetch_recent_command_events_for_device(&db, &device_id, 50).await.map_err(internal_error)?;
    let alerts = fetch_recent_alerts(&db, 50).await.map_err(internal_error)?;
    Ok(Json(serde_json::json!({
        "device_id": device_id,
        "telemetry": telemetry_to_json(&telemetry),
        "commands": commands_to_json(&commands),
        "alerts": alerts_to_json(&alerts.into_iter().filter(|alert| alert.device_id == device_id).collect::<Vec<_>>()),
    })))
}

async fn api_recent_alerts(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let db = required_db(&state)?;
    let alerts = fetch_recent_alerts(&db, 100).await.map_err(internal_error)?;
    Ok(Json(serde_json::json!(alerts_to_json(&alerts))))
}

async fn api_recent_commands(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let db = required_db(&state)?;
    let commands = fetch_recent_command_events(&db, 100).await.map_err(internal_error)?;
    Ok(Json(serde_json::json!(commands_to_json(&commands))))
}

fn required_db(state: &Arc<AppState>) -> Result<Db, (StatusCode, Json<serde_json::Value>)> {
    state.db.clone().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({"error": "database_not_configured"})),
    ))
}

fn internal_error(error: anyhow::Error) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": error.to_string()})),
    )
}

#[derive(Serialize)]
struct DroneView {
    device_id: String,
    last_seen_ms: i64,
    online: bool,
    battery_voltage_v: Option<f32>,
    altitude_m: Option<f32>,
    attitude_deg: Option<f32>,
    gps_lat: Option<f64>,
    gps_lon: Option<f64>,
    latest_command: Option<CommandEventView>,
}

#[derive(Serialize)]
struct CommandEventView {
    device_id: String,
    message_id: String,
    correlation_id: Option<String>,
    timestamp_ms: i64,
    command_kind: String,
    status: String,
    detail: Option<String>,
    ack_timestamp_ms: Option<i64>,
}

fn telemetry_to_json(rows: &[LatestTelemetryRow]) -> Vec<serde_json::Value> {
    rows.iter().map(|row| serde_json::json!({
        "device_id": row.device_id,
        "timestamp_ms": row.timestamp_ms,
        "battery_voltage_v": row.battery_voltage,
        "altitude_m": row.altitude,
        "attitude_deg": row.attitude,
        "gps_lat": row.gps_lat,
        "gps_lon": row.gps_lon,
    })).collect()
}

fn commands_to_json(rows: &[CommandEventRow]) -> Vec<serde_json::Value> {
    rows.iter().map(|row| serde_json::json!({
        "device_id": row.device_id,
        "message_id": row.message_id,
        "correlation_id": row.correlation_id,
        "timestamp_ms": row.timestamp_ms,
        "command_kind": row.command_kind,
        "status": row.status,
        "detail": row.detail,
        "ack_timestamp_ms": row.ack_timestamp_ms,
    })).collect()
}

fn alerts_to_json(rows: &[AlertRow]) -> Vec<serde_json::Value> {
    rows.iter().map(|row| serde_json::json!({
        "device_id": row.device_id,
        "message_id": row.message_id,
        "timestamp_ms": row.timestamp_ms,
        "rule_kind": row.rule_kind,
        "message": row.message,
    })).collect()
}

fn merge_drones(telemetry: &[LatestTelemetryRow], commands: &[CommandEventRow]) -> Vec<DroneView> {
    let mut latest_command_by_device: HashMap<String, &CommandEventRow> = HashMap::new();
    for command in commands {
        latest_command_by_device.entry(command.device_id.clone()).or_insert(command);
    }

    let now_ms = now_ms();
    telemetry
        .iter()
        .map(|row| {
            let latest_command = latest_command_by_device.get(&row.device_id).map(|command| CommandEventView {
                device_id: command.device_id.clone(),
                message_id: command.message_id.clone(),
                correlation_id: command.correlation_id.clone(),
                timestamp_ms: command.timestamp_ms,
                command_kind: command.command_kind.clone(),
                status: command.status.clone(),
                detail: command.detail.clone(),
                ack_timestamp_ms: command.ack_timestamp_ms,
            });

            DroneView {
                device_id: row.device_id.clone(),
                last_seen_ms: row.timestamp_ms,
                online: now_ms.saturating_sub(row.timestamp_ms) <= 120_000,
                battery_voltage_v: row.battery_voltage,
                altitude_m: row.altitude,
                attitude_deg: row.attitude,
                gps_lat: row.gps_lat,
                gps_lon: row.gps_lon,
                latest_command,
            }
        })
        .collect()
}

fn build_summary(
    telemetry: &[LatestTelemetryRow],
    alerts: &[AlertRow],
    commands: &[CommandEventRow],
) -> serde_json::Value {
    let now_ms = now_ms();
    let online_drones = telemetry.iter().filter(|row| now_ms.saturating_sub(row.timestamp_ms) <= 120_000).count();
    let total_drones = telemetry.len();
    let active_drones = commands.iter().filter(|command| command.status == "queued" || command.status == "acknowledged").count();

    serde_json::json!({
        "total_drones": total_drones,
        "online_drones": online_drones,
        "offline_drones": total_drones.saturating_sub(online_drones),
        "active_drones": active_drones,
        "idle_drones": total_drones.saturating_sub(active_drones),
        "recent_alerts": alerts.len(),
        "recent_commands": commands.len(),
        "last_telemetry_ms": telemetry.iter().map(|row| row.timestamp_ms).max(),
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
