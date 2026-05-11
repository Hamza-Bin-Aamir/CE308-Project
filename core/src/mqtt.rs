use anyhow::{Context, Result};
use crate::{pipeline::AlertOutcome, telemetry::TelemetryReading};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::{Deserialize, Serialize};
use std::{future::Future, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};

#[derive(Debug, Clone)]
pub struct MqttConfig {
    pub broker_host: String,
    pub broker_port: u16,
    pub client_id: String,
    pub swarm_id: String,
    pub topic_prefix: String,
    pub keep_alive_secs: u64,
    pub queue_capacity: usize,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl MqttConfig {
    pub fn from_env(default_client_id: &str) -> Option<Self> {
        let broker_host = std::env::var("MQTT_BROKER_HOST").ok()?;
        let broker_port = std::env::var("MQTT_BROKER_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(1883);

        let client_id = std::env::var("MQTT_CLIENT_ID").unwrap_or_else(|_| default_client_id.to_string());
        let swarm_id = std::env::var("MQTT_SWARM_ID").unwrap_or_else(|_| "demo-swarm".to_string());
        let topic_prefix = std::env::var("MQTT_TOPIC_PREFIX").unwrap_or_else(|_| "ce308".to_string());
        let keep_alive_secs = std::env::var("MQTT_KEEPALIVE_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(30);
        let queue_capacity = std::env::var("MQTT_QUEUE_CAPACITY")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(16);
        let username = std::env::var("MQTT_USERNAME").ok();
        let password = std::env::var("MQTT_PASSWORD").ok();

        Some(Self {
            broker_host,
            broker_port,
            client_id,
            swarm_id,
            topic_prefix,
            keep_alive_secs,
            queue_capacity,
            username,
            password,
        })
    }
}

#[derive(Debug, Clone)]
pub struct TopicMap {
    pub topic_prefix: String,
    pub swarm_id: String,
}

impl TopicMap {
    pub fn new(topic_prefix: impl Into<String>, swarm_id: impl Into<String>) -> Self {
        Self {
            topic_prefix: topic_prefix.into(),
            swarm_id: swarm_id.into(),
        }
    }

    fn base(&self) -> String {
        format!("{}/swarm/{}", self.topic_prefix, self.swarm_id)
    }

    pub fn telemetry_filter(&self) -> String {
        format!("{}/devices/+/telemetry", self.base())
    }

    pub fn command_filter(&self) -> String {
        format!("{}/devices/+/command", self.base())
    }

    pub fn ack_filter(&self) -> String {
        format!("{}/devices/+/ack", self.base())
    }

    pub fn command(&self, device_id: &str) -> String {
        format!("{}/devices/{}/command", self.base(), device_id)
    }

    pub fn telemetry(&self, device_id: &str) -> String {
        format!("{}/devices/{}/telemetry", self.base(), device_id)
    }

    pub fn ack(&self, device_id: &str) -> String {
        format!("{}/devices/{}/ack", self.base(), device_id)
    }

    pub fn alert(&self, device_id: &str) -> String {
        format!("{}/devices/{}/alert", self.base(), device_id)
    }

    pub fn device_id_from_topic(&self, topic: &str) -> Option<String> {
        let parts: Vec<&str> = topic.split('/').collect();
        let device_index = parts.iter().position(|segment| *segment == "devices")?;
        parts.get(device_index + 1).map(|value| (*value).to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SwarmCommand {
    Ping,
    Start,
    Stop,
    ReturnHome,
    SetAltitude { altitude_m: f32 },
    SetMission { mission: String },
}

impl SwarmCommand {
    pub fn kind_name(&self) -> &'static str {
        match self {
            SwarmCommand::Ping => "ping",
            SwarmCommand::Start => "start",
            SwarmCommand::Stop => "stop",
            SwarmCommand::ReturnHome => "return_home",
            SwarmCommand::SetAltitude { .. } => "set_altitude",
            SwarmCommand::SetMission { .. } => "set_mission",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TelemetryEnvelope {
    pub swarm_id: String,
    pub device_id: String,
    pub message_id: String,
    pub timestamp_ms: u64,
    pub reading: TelemetryReading,
}

impl TelemetryEnvelope {
    pub fn new(swarm_id: impl Into<String>, device_id: impl Into<String>, reading: TelemetryReading) -> Self {
        let swarm_id = swarm_id.into();
        let device_id = device_id.into();
        let timestamp_ms = reading.timestamp_ms;

        Self {
            message_id: message_id(&device_id),
            swarm_id,
            device_id,
            timestamp_ms,
            reading,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandEnvelope {
    pub swarm_id: String,
    pub device_id: String,
    pub message_id: String,
    pub correlation_id: Option<String>,
    pub timestamp_ms: u64,
    pub command: SwarmCommand,
}

impl CommandEnvelope {
    pub fn new(
        swarm_id: impl Into<String>,
        device_id: impl Into<String>,
        command: SwarmCommand,
        correlation_id: Option<String>,
    ) -> Self {
        let swarm_id = swarm_id.into();
        let device_id = device_id.into();

        Self {
            message_id: message_id(&device_id),
            swarm_id,
            device_id,
            correlation_id,
            timestamp_ms: now_ms(),
            command,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AckEnvelope {
    pub swarm_id: String,
    pub device_id: String,
    pub message_id: String,
    pub correlation_id: String,
    pub timestamp_ms: u64,
    pub accepted: bool,
    pub detail: Option<String>,
}

impl AckEnvelope {
    pub fn accepted(
        swarm_id: impl Into<String>,
        device_id: impl Into<String>,
        correlation_id: impl Into<String>,
        detail: impl Into<Option<String>>,
    ) -> Self {
        let swarm_id = swarm_id.into();
        let device_id = device_id.into();

        Self {
            message_id: message_id(&device_id),
            swarm_id,
            device_id,
            correlation_id: correlation_id.into(),
            timestamp_ms: now_ms(),
            accepted: true,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertEnvelope {
    pub swarm_id: String,
    pub device_id: String,
    pub message_id: String,
    pub timestamp_ms: u64,
    pub outcome: AlertOutcome,
}

impl AlertEnvelope {
    pub fn new(swarm_id: impl Into<String>, device_id: impl Into<String>, outcome: AlertOutcome) -> Self {
        let swarm_id = swarm_id.into();
        let device_id = device_id.into();

        Self {
            message_id: message_id(&device_id),
            swarm_id,
            device_id,
            timestamp_ms: now_ms(),
            outcome,
        }
    }
}

#[derive(Clone)]
pub struct MqttHandle {
    client: AsyncClient,
    topics: TopicMap,
    swarm_id: String,
}

impl MqttHandle {
    pub async fn publish_command(
        &self,
        device_id: &str,
        command: SwarmCommand,
    ) -> Result<CommandEnvelope> {
        let envelope = CommandEnvelope::new(self.swarm_id.clone(), device_id.to_string(), command, None);
        let payload = serde_json::to_vec(&envelope).context("serializing MQTT command envelope")?;

        self.client
            .publish(self.topics.command(device_id), QoS::AtLeastOnce, false, payload)
            .await
            .context("publishing MQTT command")?;

        Ok(envelope)
    }

    pub fn command_topic(&self, device_id: &str) -> String {
        self.topics.command(device_id)
    }
}

pub async fn connect_controller<H, Fut>(config: MqttConfig, on_telemetry: H) -> Result<MqttHandle>
where
    H: Fn(TelemetryEnvelope) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<AlertOutcome>> + Send + 'static,
{
    let topics = TopicMap::new(config.topic_prefix.clone(), config.swarm_id.clone());
    let mut options = MqttOptions::new(config.client_id.clone(), config.broker_host.clone(), config.broker_port);
    options.set_keep_alive(Duration::from_secs(config.keep_alive_secs));

    // Set credentials if provided
    if let (Some(username), Some(password)) = (&config.username, &config.password) {
        options.set_credentials(username, password);
    }

    // Enable TLS for port 8883 or explicitly
    if config.broker_port == 8883 {
        options.set_transport(rumqttc::Transport::tls_with_default_config());
    }

    let (client, eventloop) = AsyncClient::new(options, config.queue_capacity);
    client
        .subscribe(topics.telemetry_filter(), QoS::AtLeastOnce)
        .await
        .context("subscribing to telemetry topics")?;
    client
        .subscribe(topics.ack_filter(), QoS::AtLeastOnce)
        .await
        .context("subscribing to ack topics")?;

    let callback = Arc::new(on_telemetry);
    let loop_client = client.clone();
    let loop_topics = topics.clone();
    let loop_swarm_id = config.swarm_id.clone();

    tokio::spawn(async move {
        controller_loop(eventloop, loop_client, loop_topics, loop_swarm_id, callback).await;
    });

    Ok(MqttHandle {
        client,
        topics,
        swarm_id: config.swarm_id,
    })
}

async fn controller_loop<H, Fut>(
    mut eventloop: rumqttc::EventLoop,
    client: AsyncClient,
    topics: TopicMap,
    swarm_id: String,
    on_telemetry: Arc<H>,
)
where
    H: Fn(TelemetryEnvelope) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<AlertOutcome>> + Send + 'static,
{
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(publish))) => {
                let topic = publish.topic.clone();
                let payload = publish.payload.to_vec();

                if topic.ends_with("/telemetry") {
                    match decode_telemetry(&topics, &topic, &payload, &swarm_id) {
                        Some(envelope) => match on_telemetry(envelope.clone()).await {
                            Ok(outcome) => {
                                if !outcome.emitted_alerts.is_empty() {
                                    let alert = AlertEnvelope::new(swarm_id.clone(), envelope.device_id.clone(), outcome);
                                    if let Ok(bytes) = serde_json::to_vec(&alert) {
                                        let _ = client
                                            .publish(topics.alert(&envelope.device_id), QoS::AtLeastOnce, false, bytes)
                                            .await;
                                    }
                                }
                            }
                            Err(error) => {
                                eprintln!("mqtt telemetry handler failed: {error}");
                            }
                        },
                        None => {
                            eprintln!("mqtt controller received invalid telemetry payload on {topic}");
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("mqtt controller event loop error: {error}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

fn decode_telemetry(
    topics: &TopicMap,
    topic: &str,
    payload: &[u8],
    swarm_id: &str,
) -> Option<TelemetryEnvelope> {
    if let Ok(envelope) = serde_json::from_slice::<TelemetryEnvelope>(payload) {
        return Some(envelope);
    }

    let reading = serde_json::from_slice::<TelemetryReading>(payload).ok()?;
    let device_id = topics.device_id_from_topic(topic).unwrap_or_else(|| reading.device_id.clone());

    Some(TelemetryEnvelope::new(swarm_id.to_string(), device_id, reading))
}

fn message_id(device_id: &str) -> String {
    format!("{}-{}", device_id, now_ms())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_expected_topics() {
        let topics = TopicMap::new("ce308", "demo");

        assert_eq!(topics.command("uav-1"), "ce308/swarm/demo/devices/uav-1/command");
        assert_eq!(topics.telemetry("uav-1"), "ce308/swarm/demo/devices/uav-1/telemetry");
        assert_eq!(topics.ack("uav-1"), "ce308/swarm/demo/devices/uav-1/ack");
        assert_eq!(topics.alert("uav-1"), "ce308/swarm/demo/devices/uav-1/alert");
        assert_eq!(topics.telemetry_filter(), "ce308/swarm/demo/devices/+/telemetry");
    }

    #[test]
    fn round_trips_command_envelope() {
        let envelope = CommandEnvelope::new(
            "demo-swarm",
            "uav-7",
            SwarmCommand::SetAltitude { altitude_m: 42.5 },
            Some("cmd-123".to_string()),
        );

        let encoded = serde_json::to_string(&envelope).expect("encode command envelope");
        let decoded: CommandEnvelope = serde_json::from_str(&encoded).expect("decode command envelope");

        assert_eq!(decoded.swarm_id, "demo-swarm");
        assert_eq!(decoded.device_id, "uav-7");
        assert_eq!(decoded.correlation_id.as_deref(), Some("cmd-123"));
        assert_eq!(decoded.command, SwarmCommand::SetAltitude { altitude_m: 42.5 });
    }
}
