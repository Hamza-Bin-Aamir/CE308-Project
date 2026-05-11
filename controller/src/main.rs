use axum::{extract::State, http::StatusCode, routing::{get, post}, Json, Router};
use ce308_core::{dedupe::AlertDedupe, pipeline, telemetry::TelemetryReading};
use ce308_core::mqtt;
use redis_adapter::{create_pool_from_url, set_alert_key, RedisPool};
use storage::{init_db, insert_telemetry, Db};
use std::io::Read;
use std::sync::{Arc, Mutex};

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
        let mqtt_state = shared.clone();
        match mqtt::connect_controller(config, move |envelope| {
            let mqtt_state = mqtt_state.clone();
            async move { process_reading_with_state(mqtt_state, envelope.reading).await }
        })
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
    axum::extract::Path(device_id): axum::extract::Path<String>,
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
        tokio::spawn(async move {
            if let Err(e) = insert_telemetry(&db, &reading_clone).await {
                eprintln!("failed to persist telemetry: {e}");
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

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
