mod common;

use futures_util::{SinkExt, StreamExt};
use mqtt_controller::{
    mqtt::MqttConfig,
    time::FakeClock,
    web::{
        history::HeatingHistory,
        server::{WebHandle, WsCommand, bind_and_start_web_server},
    },
};
use mqtt_controller_wire::{ControlCommand, FullStateSnapshot, ServerMessage};
use std::{sync::Arc, time::Duration};
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::{connect_async, tungstenite::Message};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next_message(socket: &mut Socket) -> ServerMessage {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket
                .next()
                .await
                .expect("socket ended")
                .expect("socket error")
            {
                Message::Text(text) => {
                    return serde_json::from_str(&text).expect("server protocol");
                }
                Message::Ping(payload) => socket.send(Message::Pong(payload)).await.unwrap(),
                other => panic!("unexpected frame: {other:?}"),
            }
        }
    })
    .await
    .expect("server response timed out")
}

async fn command(socket: &mut Socket, request: &str, command: ControlCommand) -> Option<String> {
    socket
        .send(Message::text(
            serde_json::to_string(&mqtt_controller_wire::ClientMessage::Command {
                request_id: request.into(),
                command,
            })
            .unwrap(),
        ))
        .await
        .unwrap();
    loop {
        if let ServerMessage::CommandResult { request_id, error } = next_message(socket).await {
            assert_eq!(request_id, request);
            return error;
        }
    }
}

#[tokio::test]
async fn websocket_controls_reach_mqtt_and_reject_unknown_entities() {
    let broker = common::TestBroker::start().await;
    let mqtt = common::TestClient::connect(&broker, "web-control-observer").await;
    let topic = "zigbee2mqtt/hue-lz-kitchen-cooker/set";
    mqtt.subscribe(topic).await;
    let mut config = common::fixtures::kitchen_with_motion_config();
    config.devices.insert("test-plug".into(), serde_json::from_value(serde_json::json!({
        "kind": "plug", "ieee_address": "0x0000000000000099", "variant": "sonoff-power", "capabilities": ["power"]
    })).unwrap());
    config.devices.insert("test-relay".into(), serde_json::from_value(serde_json::json!({
        "kind": "wall-thermostat", "ieee_address": "0x0000000000000098",
        "options": { "heater_type": "manual_control", "operating_mode": "manual" }
    })).unwrap());
    config.devices.insert("test-trv".into(), serde_json::from_value(serde_json::json!({
        "kind": "trv", "ieee_address": "0x0000000000000097", "options": { "operating_mode": "manual" }
    })).unwrap());
    let days: std::collections::BTreeMap<_, _> = mqtt_controller::config::heating::Weekday::ALL
        .into_iter().map(|day| (day, serde_json::json!([{ "start": "00:00", "end": "24:00", "temperature": 20.0 }]))).collect();
    config.heating = Some(serde_json::from_value(serde_json::json!({
        "zones": [{ "name": "test-zone", "relay": "test-relay", "trvs": [{ "device": "test-trv", "schedule": "test" }] }],
        "schedules": { "test": days },
        "heat_pump": { "min_cycle_seconds": 120, "min_pause_seconds": 60, "min_demand_percent": 5, "min_demand_percent_fallback": 80 },
        "open_window": { "detection_minutes": 20, "inhibit_minutes": 80 }
    })).unwrap());
    mqtt.subscribe("zigbee2mqtt/test-plug/set").await;
    let (commands, command_rx) = mpsc::channel(64);
    let (updates, _) = broadcast::channel(256);
    let directory = tempfile::tempdir().unwrap();
    let history = HeatingHistory::open(&directory.path().join("history.db"))
        .await
        .unwrap();
    let (address, server) = bind_and_start_web_server(
        "127.0.0.1:0".parse().unwrap(),
        commands,
        updates.clone(),
        directory.path().into(),
        None,
        history,
    )
    .await
    .unwrap();
    let daemon = tokio::spawn(mqtt_controller::daemon::run(
        config,
        MqttConfig::new("127.0.0.1", broker.port, "test", "", "web-controller"),
        None,
        None,
        Arc::new(FakeClock::new(12)),
        Some(WebHandle {
            ws_cmd_rx: command_rx,
            broadcast_tx: updates,
            audit_writer: None,
        }),
        mqtt_controller::settings::SqliteSettings::open(&directory.path().join("settings.db")).await.unwrap(),
    ));
    let (mut socket, _) = connect_async(format!("ws://{address}/ws")).await.unwrap();
    assert!(matches!(
        next_message(&mut socket).await,
        ServerMessage::StateSnapshot(_)
    ));
    assert_eq!(
        command(
            &mut socket,
            "off-1",
            ControlCommand::SetRoomOff {
                room: "kitchen-cooker".into()
            }
        )
        .await,
        None
    );
    let messages = mqtt.inbox.wait_for(topic, 1, Duration::from_secs(3)).await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&messages[0]).unwrap()["state"],
        "OFF"
    );
    assert_eq!(
        command(
            &mut socket,
            "scene-2",
            ControlCommand::RecallScene {
                room: "kitchen-cooker".into(),
                scene_id: 2
            }
        )
        .await,
        None
    );
    let messages = mqtt.inbox.wait_for(topic, 2, Duration::from_secs(3)).await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&messages[1]).unwrap()["scene_recall"],
        2
    );
    for (request, on) in [
        ("plug-off", false),
        ("plug-on", true),
        ("plug-on-again", true),
    ] {
        assert_eq!(
            command(
                &mut socket,
                request,
                ControlCommand::SetPlugPower {
                    device: "test-plug".into(),
                    on
                }
            )
            .await,
            None
        );
    }
    let messages = mqtt
        .inbox
        .wait_for("zigbee2mqtt/test-plug/set", 3, Duration::from_secs(3))
        .await;
    let states: Vec<_> = messages
        .iter()
        .map(|message| {
            serde_json::from_slice::<serde_json::Value>(message).unwrap()["state"].clone()
        })
        .collect();
    assert_eq!(states, vec!["OFF", "ON", "ON"]);
    assert!(
        command(
            &mut socket,
            "bad-room",
            ControlCommand::SetRoomOff {
                room: "missing".into()
            }
        )
        .await
        .unwrap()
        .contains("Unknown light group")
    );
    assert!(
        command(
            &mut socket,
            "bad-scene",
            ControlCommand::RecallScene {
                room: "kitchen-cooker".into(),
                scene_id: 99
            }
        )
        .await
        .unwrap()
        .contains("Unknown scene")
    );
    assert_eq!(command(&mut socket, "motion-off", ControlCommand::SetMotionEnabled {
        room: "kitchen-cooker".into(), enabled: false,
    }).await, None);
    for request in ["demand-off", "demand-off-again"] {
        assert_eq!(command(&mut socket, request, ControlCommand::SetHeatDemandEnabled {
            device: "test-trv".into(), enabled: false,
        }).await, None);
    }
    assert!(command(&mut socket, "invalid-valve", ControlCommand::SetHeatDemandEnabled {
        device: "test-relay".into(), enabled: false,
    }).await.is_some());
    use mqtt_controller::settings::SettingsRepository;
    let saved = mqtt_controller::settings::SqliteSettings::open(&directory.path().join("settings.db")).await.unwrap();
    assert_eq!(command(&mut socket, "boost", ControlCommand::StartValveBoost {
        device: "test-trv".into(), duration_minutes: 90, temperature: 22.0,
    }).await, None);
    let boost = saved.load().await.unwrap().boosts["test-trv"];
    assert_eq!(command(&mut socket, "boost-target", ControlCommand::SetValveBoostTarget {
        device: "test-trv".into(), temperature: 23.5,
    }).await, None);
    let edited = saved.load().await.unwrap().boosts["test-trv"];
    assert_eq!(edited.temperature, 23.5);
    assert_eq!(edited.ends_at_epoch_ms, boost.ends_at_epoch_ms);
    assert!(command(&mut socket, "boost-bad-target", ControlCommand::SetValveBoostTarget {
        device: "test-trv".into(), temperature: 50.0,
    }).await.unwrap().contains("5–30"));
    socket.send(Message::text(r#"{"type":"GetHeatingEnergyHistory","request_id":"energy"}"#)).await.unwrap();
    loop {
        if let ServerMessage::HeatingEnergyHistory { request_id, from_epoch_ms, to_epoch_ms, points, error } = next_message(&mut socket).await {
            assert_eq!(request_id, "energy");
            assert_eq!(to_epoch_ms - from_epoch_ms, mqtt_controller::web::history::HISTORY_WINDOW_MS);
            assert!(points.is_empty());
            assert_eq!(error.as_deref(), Some("Waiting for the first history sample"));
            break;
        }
    }
    socket.close(None).await.unwrap();
    let (mut replacement, _) = connect_async(format!("ws://{address}/ws")).await.unwrap();
    let ServerMessage::StateSnapshot(snapshot) = next_message(&mut replacement).await else {
        panic!("expected reconnect snapshot")
    };
    assert!(!snapshot.rooms.iter().find(|room| room.name == "kitchen-cooker").unwrap().motion_enabled);
    assert!(!snapshot.heating_zones[0].trvs[0].heat_demand_enabled);
    let restored_boost = snapshot.heating_zones[0].trvs[0].boost.as_ref().unwrap();
    assert_eq!(restored_boost.temperature, 23.5);
    assert_eq!(restored_boost.ends_at_epoch_ms, boost.ends_at_epoch_ms);
    assert_eq!(command(&mut replacement, "boost-cancel", ControlCommand::CancelValveBoost { device: "test-trv".into() }).await, None);
    assert!(saved.load().await.unwrap().boosts.is_empty());
    replacement.close(None).await.unwrap();
    daemon.abort();
    let _ = daemon.await;
    let saved = mqtt_controller::settings::SqliteSettings::open(&directory.path().join("settings.db")).await.unwrap();
    assert!(saved.load().await.unwrap().disabled_zones.contains("kitchen-cooker"));
    assert!(saved.load().await.unwrap().disabled_heat_demand.contains("test-trv"));
    server.abort();
}

#[tokio::test]
async fn lost_broadcasts_close_the_connection_to_require_a_fresh_snapshot() {
    let (commands, mut command_rx) = mpsc::channel(1);
    let (updates, _) = broadcast::channel(1);
    let directory = tempfile::tempdir().unwrap();
    let history = HeatingHistory::open(&directory.path().join("history.db"))
        .await
        .unwrap();
    let (address, server) = bind_and_start_web_server(
        "127.0.0.1:0".parse().unwrap(),
        commands,
        updates.clone(),
        directory.path().into(),
        None,
        history,
    )
    .await
    .unwrap();
    let (mut socket, _) = connect_async(format!("ws://{address}/ws")).await.unwrap();
    let WsCommand::RequestSnapshot { reply } = command_rx.recv().await.unwrap() else {
        panic!("expected snapshot request")
    };
    let snapshot: FullStateSnapshot = serde_json::from_value(
        serde_json::json!({ "timestamp_epoch_ms": 0, "rooms": [], "plugs": [] }),
    )
    .unwrap();
    for _ in 0..8 {
        updates
            .send(ServerMessage::StateSnapshot(snapshot.clone()))
            .unwrap();
    }
    reply.send(snapshot).unwrap();
    assert!(matches!(
        next_message(&mut socket).await,
        ServerMessage::StateSnapshot(_)
    ));
    let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(frame, Message::Close(Some(ref close)) if u16::from(close.code) == 1013),
        "expected resync close, got {frame:?}"
    );
    server.abort();
}

#[tokio::test]
async fn heartbeats_are_not_blocked_by_a_pending_controller_command() {
    let (commands, mut command_rx) = mpsc::channel(2);
    let (updates, _) = broadcast::channel(16);
    let directory = tempfile::tempdir().unwrap();
    let history = HeatingHistory::open(&directory.path().join("history.db"))
        .await
        .unwrap();
    let (address, server) = bind_and_start_web_server(
        "127.0.0.1:0".parse().unwrap(),
        commands,
        updates,
        directory.path().into(),
        None,
        history,
    )
    .await
    .unwrap();
    let (mut socket, _) = connect_async(format!("ws://{address}/ws")).await.unwrap();
    let WsCommand::RequestSnapshot { reply } = command_rx.recv().await.unwrap() else {
        panic!("expected snapshot request")
    };
    reply
        .send(
            serde_json::from_value(
                serde_json::json!({ "timestamp_epoch_ms": 0, "rooms": [], "plugs": [] }),
            )
            .unwrap(),
        )
        .unwrap();
    next_message(&mut socket).await;
    socket.send(Message::text(r#"{"type":"Command","request_id":"pending","command":{"kind":"SetRoomOff","room":"kitchen"}}"#)).await.unwrap();
    let pending_command = command_rx.recv().await.unwrap();
    socket
        .send(Message::text(
            r#"{"type":"Ping","nonce":"probe","client_ts_ms":123}"#,
        ))
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_millis(500), next_message(&mut socket))
        .await
        .expect("heartbeat was blocked waiting for a controller command");
    assert!(
        matches!(response, ServerMessage::Pong { nonce, client_ts_ms: 123, .. } if nonce == "probe")
    );
    drop(pending_command);
    socket.close(None).await.unwrap();
    server.abort();
}
