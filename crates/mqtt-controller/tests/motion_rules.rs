//! Behavioral blackbox tests for independently scheduled motion rules.
use std::sync::Arc;
use std::time::Duration;

use mqtt_controller::config::Config;
use mqtt_controller::domain::event::Event;
use mqtt_controller::logic::EventProcessor;
use mqtt_controller::time::{Clock, FakeClock};
use mqtt_controller::topology::Topology;
use serde_json::json;

fn config() -> Config {
    let scenes = json!({
        "scenes": [{"id": 1, "name": "warm", "state": "ON", "brightness": 160,
                    "color_temp": 454, "transition": 0.5}],
        "slots": {
            "morning": {"from": "05:00", "to": "06:00", "scene_ids": [1]},
            "day": {"from": "06:00", "to": "18:00", "scene_ids": [1]},
            "evening": {"from": "18:00", "to": "22:00", "scene_ids": [1]},
            "night": {"from": "22:00", "to": "05:00", "scene_ids": [1]}
        }
    });
    serde_json::from_value(json!({
        "devices": {
            "wall": {"kind": "light", "ieee_address": "0xa"},
            "ceiling": {"kind": "light", "ieee_address": "0xb"},
            "sensor": {"kind": "motion-sensor", "ieee_address": "0xc",
                       "occupancy_timeout_seconds": 180}
        },
        "rooms": [{
            "name": "bathroom", "group_name": "bathroom-all", "room": "bathroom", "id": 1,
            "members": ["wall/11", "ceiling/11"], "scenes": scenes,
            "off_transition_seconds": 0.8
        }],
        "motion_rules": [{
            "name": "bathroom-motion", "sensors": ["sensor"], "mode": "on-off",
            "scenes": scenes, "off_transition_seconds": 0.8, "off_cooldown_seconds": 0,
            "targets_by_slot": {
                "day": {"group": "bathroom"},
                "evening": {"lights": ["wall/11"]},
                "night": {"lights": ["wall/11"]},
                "morning": {"lights": ["wall/11"]}
            }
        }]
    }))
    .expect("standalone motion rules must deserialize")
}

#[test]
fn wall_only_motion_keeps_its_target_across_a_schedule_boundary() {
    let cfg = config();
    let topology = Arc::new(Topology::build(&cfg).unwrap());
    let clock = Arc::new(FakeClock::new(23));
    let mut processor = EventProcessor::new(topology.clone(), clock.clone(), cfg.defaults, None);
    let on = processor.handle_event(Event::Occupancy {
        received_at_epoch_ms: Some(clock.epoch_millis()),
        sensor: "sensor".into(),
        occupied: true,
        illuminance: Some(0),
        ts: clock.now(),
    });
    assert_eq!(on.len(), 1);
    assert_eq!(on[0].topic(&topology), "zigbee2mqtt/wall/11/set");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&on[0].payload_string()).unwrap(),
        json!({"state": "ON", "brightness": 160, "color_temp": 454, "transition": 0.5})
    );
    clock.set_hour(7);
    clock.advance(Duration::from_secs(180));
    let off = processor.handle_event(Event::Occupancy {
        received_at_epoch_ms: Some(clock.epoch_millis()),
        sensor: "sensor".into(),
        occupied: false,
        illuminance: Some(0),
        ts: clock.now(),
    });
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].topic(&topology), "zigbee2mqtt/wall/11/set");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&off[0].payload_string()).unwrap(),
        json!({"state": "OFF", "transition": 0.8})
    );
}

fn processor(cfg: Config, hour: u8) -> (EventProcessor, Arc<Topology>, Arc<FakeClock>) {
    let topology = Arc::new(Topology::build(&cfg).unwrap());
    let clock = Arc::new(FakeClock::new(hour));
    (
        EventProcessor::new(topology.clone(), clock.clone(), cfg.defaults, None),
        topology,
        clock,
    )
}

fn occupancy(clock: &FakeClock, occupied: bool) -> Event {
    Event::Occupancy {
        received_at_epoch_ms: Some(clock.epoch_millis()),
        sensor: "sensor".into(),
        occupied,
        illuminance: Some(0),
        ts: clock.now(),
    }
}

#[test]
fn snapshot_shows_latest_motion_event_time_and_kind() {
    let (mut p, _, clock) = processor(config(), 12);
    p.set_motion_enabled("bathroom", false, clock.now()).unwrap();
    for (occupied, kind) in [(true, "motion"), (true, "motion"), (false, "clear")] {
        clock.advance(Duration::from_secs(10));
        p.handle_event(occupancy(&clock, occupied));
        let snapshot = mqtt_controller::web::snapshot::build_room_snapshot(&p, "bathroom", clock.now()).unwrap();
        let value = serde_json::to_value(snapshot).unwrap();
        assert_eq!(value["motion_rules"][0]["sensors"][0]["last_event"], json!({
            "timestamp_epoch_ms": clock.epoch_millis(), "kind": kind,
        }));
    }
}

#[test]
fn cached_state_ticks_and_dispatch_delay_do_not_fabricate_motion_event_times() {
    let (mut p, _, clock) = processor(config(), 12);
    p.set_motion_enabled("bathroom", false, clock.now()).unwrap();
    let last_event = |p: &EventProcessor| {
        mqtt_controller::web::snapshot::build_room_snapshot(p, "bathroom", clock.now())
            .unwrap().motion_rules[0].sensors[0].last_event.clone()
    };
    assert_eq!(last_event(&p), None);
    let cached = |occupied| Event::Occupancy {
        sensor: "sensor".into(), occupied, illuminance: None,
        received_at_epoch_ms: None, ts: clock.now(),
    };
    p.handle_event(cached(true));
    assert_eq!(last_event(&p), None);
    let queued = occupancy(&clock, false);
    let received_at = clock.epoch_millis();
    clock.advance(Duration::from_secs(30));
    p.handle_event(queued);
    let expected = mqtt_controller_wire::MotionEventInfo {
        timestamp_epoch_ms: received_at, kind: mqtt_controller_wire::MotionEventKind::Clear,
    };
    assert_eq!(last_event(&p), Some(expected.clone()));
    clock.advance(Duration::from_secs(3600));
    p.handle_event(Event::Tick { ts: clock.now() });
    assert_eq!(last_event(&p), Some(expected.clone()));
    p.handle_event(cached(true));
    assert_eq!(last_event(&p), Some(expected));
}

#[test]
fn motion_toggle_protocol_and_initial_state() {
    let command = json!({"kind": "SetMotionEnabled", "room": "bathroom", "enabled": false});
    serde_json::from_value::<mqtt_controller_wire::ControlCommand>(command)
        .expect("dashboard must accept a per-zone motion toggle");
    let (p, _, clock) = processor(config(), 12);
    let snapshot = mqtt_controller::web::snapshot::build_room_snapshot(&p, "bathroom", clock.now()).unwrap();
    assert_eq!(serde_json::to_value(snapshot).unwrap()["motion_enabled"], true);
}

#[test]
fn disabled_motion_suppresses_all_modes_but_preserves_sensor_reports_and_manual_controls() {
    use mqtt_controller::config::MotionMode;
    for mode in [MotionMode::OnOff, MotionMode::OnOnly, MotionMode::OffOnly] {
        let mut cfg = config();
        cfg.motion_rules[0].mode = mode;
        let (mut p, _, clock) = processor(cfg, 12);
        p.set_motion_enabled("bathroom", false, clock.now()).unwrap();
        assert!(p.handle_event(occupancy(&clock, true)).is_empty());
        let snapshot = mqtt_controller::web::snapshot::build_room_snapshot(&p, "bathroom", clock.now()).unwrap();
        assert!(!snapshot.motion_enabled);
        assert_eq!(snapshot.motion_rules[0].sensors[0].occupied, Some(true));
        assert_eq!(p.web_recall_scene("bathroom", 1, clock.now()).len(), 1);
        assert!(p.handle_event(occupancy(&clock, false)).is_empty());
        assert_eq!(p.web_set_room_off("bathroom", clock.now()).len(), 1);
    }
}

#[test]
fn disabling_active_motion_releases_claims_without_changing_lights() {
    use mqtt_controller::config::MotionMode;
    for mode in [MotionMode::OnOff, MotionMode::OnOnly, MotionMode::OffOnly] {
        for stale in [false, true] {
            let mut cfg = config();
            cfg.motion_rules[0].mode = mode;
            let (mut p, _, clock) = processor(cfg, 12);
            p.handle_event(occupancy(&clock, true));
            let before = p.world().lights["wall"].target.value().cloned();
            p.set_motion_enabled("bathroom", false, clock.now()).unwrap();
            assert_eq!(p.world().lights["wall"].target.value(), before.as_ref());
            assert!(p.world().motion_rules["bathroom-motion"].session.is_none());
            // Off-only must no longer claim subsequent manual commands.
            p.web_recall_scene("bathroom", 1, clock.now());
            let effects = if stale {
                clock.advance(Duration::from_secs(3600));
                p.handle_event(Event::Tick { ts: clock.now() })
            } else {
                p.handle_event(occupancy(&clock, false))
            };
            assert!(effects.is_empty(), "mode={mode:?}, stale={stale}");
        }
    }
}

fn overlapping_config() -> Config {
    let mut cfg = config();
    let mut wall_zone = cfg.rooms[0].clone();
    wall_zone.name = "wall-zone".into();
    wall_zone.group_name = "wall-group".into();
    wall_zone.id = 2;
    wall_zone.members = vec!["wall/11".into()];
    cfg.rooms.push(wall_zone);
    cfg
}

#[test]
fn disabling_overlapping_zone_filters_group_commands_and_active_sessions() {
    for already_active in [false, true] {
        let (mut p, topology, clock) = processor(overlapping_config(), 12);
        if already_active {
            assert_eq!(p.handle_event(occupancy(&clock, true)).len(), 1);
        }
        p.set_motion_enabled("wall-zone", false, clock.now()).unwrap();
        if !already_active {
            // A disabled member being ON must not block the other members.
            p.set_zone_actual("bathroom", true, clock.now());
            let effects = p.handle_event(occupancy(&clock, true));
            assert_eq!(effects.len(), 1);
            assert_eq!(effects[0].topic(&topology), "zigbee2mqtt/ceiling/11/set");
        }
        let effects = p.handle_event(occupancy(&clock, false));
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].topic(&topology), "zigbee2mqtt/ceiling/11/set");
    }
}

#[test]
fn reenable_waits_for_a_new_occupancy_transition() {
    let (mut p, _, clock) = processor(config(), 12);
    p.set_motion_enabled("bathroom", false, clock.now()).unwrap();
    assert!(p.handle_event(occupancy(&clock, true)).is_empty());
    p.set_motion_enabled("bathroom", true, clock.now()).unwrap();
    assert!(p.handle_event(occupancy(&clock, true)).is_empty());
    assert!(p.handle_event(occupancy(&clock, false)).is_empty());
    assert_eq!(p.handle_event(occupancy(&clock, true)).len(), 1);
}

#[test]
fn startup_reset_excludes_disabled_members_of_a_group() {
    let (mut p, topology, clock) = processor(overlapping_config(), 12);
    p.set_motion_enabled("wall-zone", false, clock.now()).unwrap();
    p.set_zone_actual("bathroom", true, clock.now());
    let effects = p.startup_turn_off_motion_zones(clock.now());
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].topic(&topology), "zigbee2mqtt/ceiling/11/set");
}

#[test]
fn shared_sensor_keeps_other_zone_active() {
    use mqtt_controller::config::MotionTarget;
    let mut cfg = config();
    cfg.rooms[0].members = vec!["wall/11".into()];
    let mut other = cfg.rooms[0].clone();
    other.name = "other".into();
    other.group_name = "other-group".into();
    other.id = 2;
    other.members = vec!["ceiling/11".into()];
    cfg.rooms.push(other);
    let mut rule = cfg.motion_rules[0].clone();
    rule.name = "other-motion".into();
    for target in rule.targets_by_slot.values_mut() {
        *target = MotionTarget::Group { group: "other".into() };
    }
    cfg.motion_rules.push(rule);
    let (mut p, topology, clock) = processor(cfg, 12);
    p.set_motion_enabled("bathroom", false, clock.now()).unwrap();
    for occupied in [true, false] {
        let effects = p.handle_event(occupancy(&clock, occupied));
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].topic(&topology), "zigbee2mqtt/other-group/set");
    }
}

#[test]
fn toggle_rejects_zones_without_motion_sensors() {
    let mut cfg = config();
    cfg.motion_rules.clear();
    let (mut p, _, clock) = processor(cfg, 12);
    assert!(p.set_motion_enabled("bathroom", false, clock.now()).unwrap_err().contains("no motion sensors"));
    assert!(p.motion_enabled("bathroom"));
}

#[derive(Default)]
struct MemorySettings(std::sync::Mutex<mqtt_controller::settings::ControlSettings>);

impl mqtt_controller::settings::SettingsRepository for MemorySettings {
    async fn set_heat_demand_enabled(&self, device: &str, enabled: bool) -> anyhow::Result<()> {
        let mut settings = self.0.lock().unwrap();
        if enabled { settings.disabled_heat_demand.remove(device); }
        else { settings.disabled_heat_demand.insert(device.into()); }
        Ok(())
    }
    async fn load(&self) -> anyhow::Result<mqtt_controller::settings::ControlSettings> {
        Ok(self.0.lock().unwrap().clone())
    }

    async fn set_motion_enabled(&self, room: &str, enabled: bool) -> anyhow::Result<()> {
        let mut settings = self.0.lock().unwrap();
        if enabled { settings.disabled_zones.remove(room); }
        else { settings.disabled_zones.insert(room.into()); }
        Ok(())
    }
}

async fn settings_contract(repository: &impl mqtt_controller::settings::SettingsRepository) {
    repository.set_heat_demand_enabled("valve-a", false).await.unwrap();
    repository.set_heat_demand_enabled("valve-a", false).await.unwrap();
    repository.set_heat_demand_enabled("valve-b", false).await.unwrap();
    repository.set_heat_demand_enabled("valve-b", true).await.unwrap();
    assert_eq!(repository.load().await.unwrap().disabled_heat_demand.into_iter().collect::<Vec<_>>(), vec!["valve-a"]);
    assert!(repository.load().await.unwrap().disabled_zones.is_empty());
    repository.set_motion_enabled("bathroom", false).await.unwrap();
    repository.set_motion_enabled("bathroom", false).await.unwrap();
    repository.set_motion_enabled("other", false).await.unwrap();
    assert_eq!(repository.load().await.unwrap().disabled_zones.len(), 2);
    repository.set_motion_enabled("other", true).await.unwrap();
    let (mut p, _, clock) = processor(config(), 12);
    p.restore_settings(repository.load().await.unwrap());
    p.set_zone_actual("bathroom", true, clock.now());
    assert!(p.startup_turn_off_motion_zones(clock.now()).is_empty());
    assert!(p.handle_event(occupancy(&clock, true)).is_empty());
    p.handle_event(occupancy(&clock, false));
    p.set_zone_actual("bathroom", false, clock.now());
    mqtt_controller::settings::set_motion_enabled(&mut p, repository, "bathroom", true, clock.now()).await.unwrap();
    assert!(p.motion_enabled("bathroom"));
    assert!(repository.load().await.unwrap().disabled_zones.is_empty());
    assert_eq!(p.handle_event(occupancy(&clock, true)).len(), 1);
    mqtt_controller::settings::set_motion_enabled(&mut p, repository, "bathroom", false, clock.now()).await.unwrap();
    assert!(p.handle_event(occupancy(&clock, false)).is_empty());
    assert!(mqtt_controller::settings::set_motion_enabled(&mut p, repository, "unknown", false, clock.now()).await.is_err());
    assert_eq!(repository.load().await.unwrap().disabled_zones.into_iter().collect::<Vec<_>>(), vec!["bathroom"]);
    assert!(repository.load().await.unwrap().disabled_heat_demand.contains("valve-a"));
}

#[tokio::test]
async fn motion_settings_contract_with_memory_store() {
    settings_contract(&MemorySettings::default()).await;
}

#[tokio::test]
async fn motion_settings_contract_with_sqlite_and_reopen() {
    use mqtt_controller::settings::{SettingsRepository, SqliteSettings};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.db");
    let repository = SqliteSettings::open(&path).await.unwrap();
    settings_contract(&repository).await;
    drop(repository);
    let reopened = SqliteSettings::open(&path).await.unwrap();
    assert!(reopened.load().await.unwrap().disabled_heat_demand.contains("valve-a"));
    let (mut p, _, clock) = processor(config(), 12);
    p.restore_settings(reopened.load().await.unwrap());
    assert!(!p.motion_enabled("bathroom"));
    assert!(p.handle_event(occupancy(&clock, true)).is_empty());
    reopened.set_motion_enabled("bathroom", true).await.unwrap();
    drop(reopened);
    assert!(SqliteSettings::open(&path).await.unwrap().load().await.unwrap().disabled_zones.is_empty());
}

#[tokio::test]
async fn failed_save_does_not_change_runtime_setting_or_cancel_session() {
    struct UnwritableSettings;
    impl mqtt_controller::settings::SettingsRepository for UnwritableSettings {
        async fn set_heat_demand_enabled(&self, _: &str, _: bool) -> anyhow::Result<()> {
            anyhow::bail!("read-only database")
        }
        async fn load(&self) -> anyhow::Result<mqtt_controller::settings::ControlSettings> {
            Ok(Default::default())
        }
        async fn set_motion_enabled(&self, _: &str, _: bool) -> anyhow::Result<()> {
            anyhow::bail!("read-only database")
        }
    }
    let (mut p, _, clock) = processor(config(), 12);
    p.handle_event(occupancy(&clock, true));
    let error = mqtt_controller::settings::set_motion_enabled(&mut p, &UnwritableSettings, "bathroom", false, clock.now()).await.unwrap_err();
    assert!(error.contains("read-only database"));
    assert!(p.motion_enabled("bathroom"));
    assert_eq!(p.handle_event(occupancy(&clock, false)).len(), 1);
}

#[test]
fn slot_selects_wall_or_existing_group() {
    for (hour, topic) in [
        (5, "wall/11"),
        (12, "bathroom-all"),
        (19, "wall/11"),
        (23, "wall/11"),
    ] {
        let (mut p, topology, clock) = processor(config(), hour);
        let effects = p.handle_event(occupancy(&clock, true));
        assert_eq!(effects.len(), 1);
        assert_eq!(
            effects[0].topic(&topology),
            format!("zigbee2mqtt/{topic}/set")
        );
    }
}

#[test]
fn daytime_session_still_turns_off_whole_group_after_dark() {
    let (mut p, topology, clock) = processor(config(), 12);
    p.handle_event(occupancy(&clock, true));
    clock.set_hour(23);
    let effects = p.handle_event(occupancy(&clock, false));
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].topic(&topology), "zigbee2mqtt/bathroom-all/set");
}

#[test]
fn manual_scene_takes_over_wall_motion() {
    let (mut p, _, clock) = processor(config(), 23);
    p.handle_event(occupancy(&clock, true));
    assert_eq!(p.web_recall_scene("bathroom", 1, clock.now()).len(), 1);
    assert!(p.handle_event(occupancy(&clock, false)).is_empty());
}

#[test]
fn off_only_keeps_only_selected_light_after_manual_group_scene() {
    let mut cfg = config();
    cfg.motion_rules[0].mode = mqtt_controller::config::MotionMode::OffOnly;
    let (mut p, topology, clock) = processor(cfg, 23);
    assert!(p.handle_event(occupancy(&clock, true)).is_empty());
    p.web_recall_scene("bathroom", 1, clock.now());
    let effects = p.handle_event(occupancy(&clock, false));
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].topic(&topology), "zigbee2mqtt/wall/11/set");
}

#[test]
fn on_only_does_not_turn_off_on_vacancy() {
    let mut cfg = config();
    cfg.motion_rules[0].mode = mqtt_controller::config::MotionMode::OnOnly;
    let (mut p, _, clock) = processor(cfg, 23);
    assert_eq!(p.handle_event(occupancy(&clock, true)).len(), 1);
    assert!(p.handle_event(occupancy(&clock, false)).is_empty());
}

#[test]
fn stale_sensor_cleanup_uses_pinned_subset() {
    let (mut p, topology, clock) = processor(config(), 23);
    p.handle_event(occupancy(&clock, true));
    clock.advance(Duration::from_secs(3600));
    clock.set_hour(12);
    let effects = p.handle_event(Event::Tick { ts: clock.now() });
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].topic(&topology), "zigbee2mqtt/wall/11/set");
}

#[test]
fn invalid_rule_targets_are_rejected() {
    use mqtt_controller::config::MotionTarget;
    let mut overlap = config();
    let mut second = overlap.motion_rules[0].clone();
    second.name = "other".into();
    overlap.motion_rules.push(second);
    assert!(
        Topology::build(&overlap)
            .unwrap_err()
            .to_string()
            .contains("also targeted")
    );
    let mut missing_slot = config();
    missing_slot.motion_rules[0]
        .targets_by_slot
        .remove("morning");
    assert!(
        Topology::build(&missing_slot)
            .unwrap_err()
            .to_string()
            .contains("exactly")
    );
    let mut unknown_light = config();
    unknown_light.motion_rules[0].targets_by_slot.insert(
        "day".into(),
        MotionTarget::Lights {
            lights: vec!["ghost/11".into()],
        },
    );
    assert!(
        Topology::build(&unknown_light)
            .unwrap_err()
            .to_string()
            .contains("unknown light")
    );
    assert!(
        serde_json::from_value::<MotionTarget>(json!({"group":"bathroom", "lights":["wall/11"]}))
            .is_err()
    );
}

#[test]
fn group_echo_does_not_confirm_individual_targets() {
    use mqtt_controller::tass::TargetPhase;
    let (mut p, _, clock) = processor(config(), 23);
    p.handle_event(occupancy(&clock, true));
    p.handle_event(Event::GroupState {
        group: "bathroom-all".into(),
        on: true,
        ts: clock.now(),
    });
    assert_eq!(
        p.world().lights["wall"].target.phase(),
        TargetPhase::Commanded
    );
    assert!(!p.world().lights.contains_key("ceiling"));
    p.handle_event(Event::LightState {
        device: "wall".into(),
        on: true,
        brightness: Some(160),
        color_temp: Some(454),
        color_xy: None,
        ts: clock.now(),
    });
    assert_eq!(
        p.world().lights["wall"].target.phase(),
        TargetPhase::Confirmed
    );
}

#[test]
#[ignore = "set MQTT_CONTROLLER_CONFIG to a rendered Nix configuration"]
fn rendered_configuration_builds_topology() {
    let path = std::env::var_os("MQTT_CONTROLLER_CONFIG").expect("MQTT_CONTROLLER_CONFIG");
    let cfg: Config = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    Topology::build(&cfg).unwrap();
}

#[test]
fn vacancy_clears_active_motion_badge() {
    let (mut p, _, clock) = processor(config(), 23);
    p.handle_event(occupancy(&clock, true));
    p.handle_event(occupancy(&clock, false));
    let room =
        mqtt_controller::web::snapshot::build_room_snapshot(&p, "bathroom", clock.now()).unwrap();
    assert!(!room.motion_owned);
    assert!(room.motion_rules[0].session_targets.is_empty());
}

mod common;

#[tokio::test]
async fn daemon_publishes_wall_commands_to_endpoint_topic() {
    let broker = common::TestBroker::start().await;
    let client = common::TestClient::connect(&broker, "motion-rule-observer").await;
    for topic in [
        "zigbee2mqtt/wall/11/set",
        "zigbee2mqtt/ceiling/11/set",
        "zigbee2mqtt/bathroom-all/set",
    ] {
        client.subscribe(topic).await;
    }
    let clock = Arc::new(FakeClock::new(23));
    let shutdown = common::spawn_daemon(config(), &broker, clock.clone());
    tokio::time::sleep(Duration::from_millis(250)).await;
    client
        .publish(
            "zigbee2mqtt/sensor",
            r#"{"occupancy":true,"illuminance":0}"#,
        )
        .await;
    let on = client
        .inbox
        .wait_for("zigbee2mqtt/wall/11/set", 1, Duration::from_secs(3))
        .await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&on[0]).unwrap(),
        json!({"state":"ON", "brightness":160, "color_temp":454, "transition":0.5})
    );
    clock.set_hour(12);
    client
        .publish(
            "zigbee2mqtt/sensor",
            r#"{"occupancy":false,"illuminance":0}"#,
        )
        .await;
    let off = client
        .inbox
        .wait_for("zigbee2mqtt/wall/11/set", 2, Duration::from_secs(3))
        .await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&off[1]).unwrap(),
        json!({"state":"OFF", "transition":0.8})
    );
    assert_eq!(client.inbox.count("zigbee2mqtt/ceiling/11/set").await, 0);
    assert_eq!(client.inbox.count("zigbee2mqtt/bathroom-all/set").await, 0);
    shutdown.send(()).await.unwrap();
}
