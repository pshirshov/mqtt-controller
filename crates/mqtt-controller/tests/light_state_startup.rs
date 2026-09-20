//! Transport regression: startup cache and unsolicited MQTT reports need no group echoes.
mod common;

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mqtt_controller::mqtt::MqttConfig;
use mqtt_controller::time::FakeClock;
use mqtt_controller::web::server::{WebHandle, WsCommand};
use mqtt_controller_wire::{EntityUpdate, RoomActualValue, ServerMessage};
use serde_json::json;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[rstest::rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn cached_bulbs_seed_zones_and_unsolicited_mqtt_updates_are_broadcast(#[case] disable_motion: bool) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let cache = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        let inventory = ["hue-l-cooker", "hue-l-dining", "hue-l-empty"]
            .map(|name| json!({"friendly_name": name, "type": "Router"}));
        for message in [
            json!({"topic": "bridge/devices", "payload": inventory}),
            json!({"topic": "bridge/groups", "payload": [{"friendly_name": "hue-lz-kitchen-all"}]}),
            json!({"topic": "hue-l-cooker", "payload": {"state": "ON"}}),
            json!({"topic": "hue-l-dining", "payload": {"state": "OFF"}}),
            json!({"topic": "hue-l-empty", "payload": {"state": "OFF"}}),
        ] {
            ws.send(Message::text(message.to_string())).await.unwrap();
        }
        let _ = ws.next().await;
    });
    let broker = common::TestBroker::start().await;
    let (commands, command_rx) = mpsc::channel(16);
    let (updates, mut update_rx) = broadcast::channel(256);
    let directory = tempfile::tempdir().unwrap();
    use mqtt_controller::settings::{SettingsRepository, SqliteSettings};
    let settings_path = directory.path().join("settings.db");
    let config = if disable_motion {
        let settings = SqliteSettings::open(&settings_path).await.unwrap();
        settings.set_motion_enabled("kitchen-cooker", false).await.unwrap();
        common::fixtures::kitchen_with_motion_config()
    } else {
        common::fixtures::kitchen_config()
    };
    let daemon = tokio::spawn(mqtt_controller::daemon::run(
        config,
        MqttConfig::new("127.0.0.1", broker.port, "test", "", "light-state-test"),
        Some(format!("ws://{address}/api")),
        None,
        Arc::new(FakeClock::new(12)),
        Some(WebHandle { ws_cmd_rx: command_rx, broadcast_tx: updates, audit_writer: None }),
        SqliteSettings::open(&settings_path).await.unwrap(),
    ));
    let (reply, response) = oneshot::channel();
    commands.send(WsCommand::RequestSnapshot { reply }).await.unwrap();
    let snapshot = tokio::time::timeout(Duration::from_secs(5), response).await.unwrap().unwrap();
    for (name, expected) in [
        ("kitchen-all", RoomActualValue::On),
        ("kitchen-cooker", RoomActualValue::On),
        ("kitchen-dining", RoomActualValue::Off),
    ] {
        let room = snapshot.rooms.iter().find(|room| room.name == name).unwrap();
        assert_eq!(room.actual_value, Some(expected), "{name}");
        assert_eq!(room.target_value, None, "{name}");
        if name == "kitchen-cooker" {
            assert_eq!(room.motion_enabled, !disable_motion);
        }
    }

    let publisher = common::TestClient::connect(&broker, "light-state-publisher").await;
    publisher.publish("zigbee2mqtt/hue-l-cooker", r#"{"state":"OFF"}"#).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut parent_off = false;
        let mut child_off = false;
        while !(parent_off && child_off) {
            if let ServerMessage::Entity(EntityUpdate::Room(room)) = update_rx.recv().await.unwrap() {
                if room.actual_value == Some(RoomActualValue::Off) {
                    parent_off |= room.name == "kitchen-all";
                    child_off |= room.name == "kitchen-cooker";
                }
            }
        }
    }).await.expect("unsolicited bulb report must broadcast both zone updates");
    daemon.abort();
    cache.await.unwrap();
}
