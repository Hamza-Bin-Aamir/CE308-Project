use ce308_core::{mqtt::{AckEnvelope, MqttConfig, TelemetryEnvelope, TopicMap}, telemetry::TelemetryReading};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Simulator runs many logical drones inside a single process and shares one MQTT connection.
/// Configure number of logical drones with `SIM_DEVICE_COUNT` or CLI first arg.
#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let desired = args.get(1)
        .and_then(|s| s.parse::<usize>().ok())
        .or_else(|| std::env::var("SIM_DEVICE_COUNT").ok().and_then(|s| s.parse::<usize>().ok()))
        .unwrap_or(1);

    let mqtt_config = match MqttConfig::from_env("ce308-sim") {
        Some(config) => config,
        None => {
            eprintln!("MQTT_BROKER_HOST is required for the simulator");
            std::process::exit(1);
        }
    };

    let interval_ms = std::env::var("SIM_TELEMETRY_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2000);

    // single topics mapping per process
    let topics = TopicMap::new(mqtt_config.topic_prefix.clone(), mqtt_config.swarm_id.clone());
    let mut options = MqttOptions::new(mqtt_config.client_id.clone(), mqtt_config.broker_host.clone(), mqtt_config.broker_port);
    options.set_keep_alive(Duration::from_secs(mqtt_config.keep_alive_secs));

    // HiveMQ Cloud typically uses username/password auth.
    if let Some(username) = mqtt_config.username.as_deref() {
        let password = mqtt_config.password.as_deref().unwrap_or("");
        options.set_credentials(username, password);
    }

    // Port 8883 implies TLS transport for MQTT over TCP.
    if mqtt_config.broker_port == 8883 {
        options.set_transport(rumqttc::Transport::tls_with_default_config());
    }

    let (client, mut eventloop) = AsyncClient::new(options, mqtt_config.queue_capacity);

    // subscribe once to all commands using a wildcard
    if let Err(error) = client.subscribe(topics.command_filter(), QoS::AtLeastOnce).await {
        eprintln!("failed to subscribe to command wildcard topic: {error}");
        std::process::exit(1);
    }

    // Per-device command channels
    let mut device_senders: HashMap<String, mpsc::Sender<ce308_core::mqtt::CommandEnvelope>> = HashMap::new();

    // Spawn N logical devices inside this process
    for i in 0..desired {
        let device_id = format!("uav-sim-{}", i+1);
        let (tx, mut rx) = mpsc::channel::<ce308_core::mqtt::CommandEnvelope>(8);
        device_senders.insert(device_id.clone(), tx);

        let device_topics = topics.clone();
        let device_client = client.clone();
        let swarm_id = mqtt_config.swarm_id.clone();
        let device_id_clone = device_id.clone();

        // spawn background task per logical drone
        tokio::spawn(async move {
            let mut seq: u64 = 0;
            // initial staggering based on device index to avoid lockstep
            let init_delay = (device_id_clone.bytes().fold(0u32, |acc, b| acc + u32::from(b)) % 500) as u64;
            tokio::time::sleep(Duration::from_millis(init_delay)).await;

            loop {
                seq = seq.saturating_add(1);
                let reading = build_reading(&device_id_clone, seq);
                let envelope = TelemetryEnvelope::new(swarm_id.clone(), device_id_clone.clone(), reading);

                if let Ok(bytes) = serde_json::to_vec(&envelope) {
                    if let Err(err) = device_client.publish(device_topics.telemetry(&device_id_clone), QoS::AtLeastOnce, false, bytes).await {
                        eprintln!("failed to publish telemetry for {device_id_clone}: {err}");
                    }
                }

                // process pending command for this device (non-blocking)
                while let Ok(cmd) = rx.try_recv() {
                    // send ack
                    let ack = AckEnvelope::accepted(
                        swarm_id.clone(),
                        device_id_clone.clone(),
                        cmd.message_id.clone(),
                        Some(format!("acknowledged {}", cmd.command.kind_name())),
                    );

                    if let Ok(bytes) = serde_json::to_vec(&ack) {
                        if let Err(err) = device_client.publish(device_topics.ack(&device_id_clone), QoS::AtLeastOnce, false, bytes).await {
                            eprintln!("failed to publish ack for {device_id_clone}: {err}");
                        }
                    }
                }

                // jittered sleep to avoid synchronized bursts
                let jitter = ((seq as u64).wrapping_mul(17) % 200) as u64;
                tokio::time::sleep(Duration::from_millis(interval_ms + jitter)).await;
            }
        });
    }

    // Event loop: route command publishes to device channels
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(publish))) => {
                let topic = publish.topic.clone();
                let payload = publish.payload.to_vec();

                if topic.ends_with("/command") {
                    if let Ok(cmd) = serde_json::from_slice::<ce308_core::mqtt::CommandEnvelope>(&payload) {
                        if let Some(sender) = device_senders.get(&cmd.device_id) {
                            // best-effort send; drop if queue full
                            let _ = sender.try_send(cmd);
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(err) => {
                eprintln!("simulator event loop error: {err}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

fn build_reading(device_id: &str, sequence: u64) -> TelemetryReading {
    let device_bias = device_id.bytes().fold(0u32, |acc, byte| acc + u32::from(byte)) as f32 / 100.0;
    let battery_voltage_v = 12.4 - (sequence as f32 * 0.04) - device_bias * 0.01;
    let altitude_m = 40.0 + ((sequence % 12) as f32 * 1.25);
    let attitude_deg = 5.0 + ((sequence % 18) as f32 * 0.5);
    let drift = sequence as f64 * 0.0001;

    TelemetryReading {
        device_id: device_id.to_string(),
        timestamp_ms: now_ms(),
        battery_voltage_v,
        altitude_m,
        attitude_deg,
        gps_lat: 33.6844 + drift,
        gps_lon: 73.0479 + drift,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}
