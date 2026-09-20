//! Behavioral blackbox contract for scheduled switch step policies.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use mqtt_controller::config::{Config, switch_model::Gesture};
use mqtt_controller::domain::{Effect, event::Event};
use mqtt_controller::logic::EventProcessor;
use mqtt_controller::time::{Clock, FakeClock};
use mqtt_controller::topology::Topology;
use mqtt_controller::web::snapshot::{build_full_snapshot, build_room_snapshot};
use serde_json::{Value, json};

fn configuration() -> Value {
    let steps: Vec<_> = [3, 2, 1]
        .into_iter()
        .flat_map(|scene_id| {
            [
                json!({"scene_id": scene_id, "lights": ["wall/11"]}),
                json!({"scene_id": scene_id, "lights": ["wall/11", "ceiling/11"]}),
            ]
        })
        .collect();
    json!({
        "defaults": {"cycle_window_seconds": 2.0},
        "devices": {
            "wall": {"kind": "light", "ieee_address": "0xa"},
            "ceiling": {"kind": "light", "ieee_address": "0xb"},
            "switch": {"kind": "switch", "ieee_address": "0xc", "model": "test"}
        },
        "switch_models": {"test": {"buttons": ["on", "off", "toggle", "tap", "up", "hold", "release"], "z2m_action_map": {}}},
        "rooms": [{
            "name": "bathroom", "room": "bathroom", "group_name": "bathroom-all", "id": 1,
            "members": ["wall/11", "ceiling/11"], "off_transition_seconds": 0.8,
            "scenes": {
                "scenes": [
                    {"id": 1, "name": "bright", "brightness": 254, "color_temp": 250, "transition": 0.5},
                    {"id": 2, "name": "warm", "brightness": 254, "color_temp": 370, "transition": 0.5},
                    {"id": 3, "name": "dim", "brightness": 160, "color_temp": 454, "transition": 0.5}
                ],
                "slots": {
                    "day": {"from": "06:00", "to": "18:00", "scene_ids": [1, 2]},
                    "evening": {"from": "18:00", "to": "23:00", "scene_ids": [3, 2, 1]},
                    "night": {"from": "23:00", "to": "05:00", "scene_ids": [3, 2, 1]},
                    "morning": {"from": "05:00", "to": "06:00", "scene_ids": [3, 2, 1]}
                }
            },
            "switch_steps": {"evening": steps, "night": steps, "morning": steps}
        }],
        "bindings": [
            {"name": "on", "trigger": {"kind": "button", "device": "switch", "button": "on", "gesture": "press"}, "effect": {"kind": "scene_cycle", "room": "bathroom"}},
            {"name": "off", "trigger": {"kind": "button", "device": "switch", "button": "off", "gesture": "press"}, "effect": {"kind": "turn_off_room", "room": "bathroom"}},
            {"name": "toggle", "trigger": {"kind": "button", "device": "switch", "button": "toggle", "gesture": "press"}, "effect": {"kind": "scene_toggle", "room": "bathroom"}},
            {"name": "tap", "trigger": {"kind": "button", "device": "switch", "button": "tap", "gesture": "press"}, "effect": {"kind": "scene_toggle_cycle", "room": "bathroom"}},
            {"name": "up", "trigger": {"kind": "button", "device": "switch", "button": "up", "gesture": "press"}, "effect": {"kind": "brightness_step", "room": "bathroom", "step": 25, "transition": 0.2}},
            {"name": "hold", "trigger": {"kind": "button", "device": "switch", "button": "hold", "gesture": "press"}, "effect": {"kind": "brightness_move", "room": "bathroom", "rate": 40}},
            {"name": "release", "trigger": {"kind": "button", "device": "switch", "button": "release", "gesture": "press"}, "effect": {"kind": "brightness_stop", "room": "bathroom"}}
        ]
    })
}

fn processor(hour: u8) -> (EventProcessor, Arc<FakeClock>) {
    configured_processor(configuration(), hour)
}

fn configured_processor(value: Value, hour: u8) -> (EventProcessor, Arc<FakeClock>) {
    let config: Config = serde_json::from_value(value).unwrap();
    let topology = Arc::new(Topology::build(&config).unwrap());
    let clock = Arc::new(FakeClock::new(hour));
    (
        EventProcessor::new(topology, clock.clone(), config.defaults, None),
        clock,
    )
}

fn observe(p: &mut EventProcessor, clock: &FakeClock, device: &str, on: bool, brightness: u8) {
    assert!(
        p.handle_event(Event::LightState {
            device: device.into(),
            on,
            brightness: Some(brightness),
            color_temp: Some(454),
            color_xy: None,
            ts: clock.now(),
        })
        .is_empty()
    );
}

fn phase(p: &EventProcessor, clock: &FakeClock) -> String {
    build_room_snapshot(p, "bathroom", clock.now())
        .unwrap()
        .target
        .unwrap()
        .phase
}

fn press(p: &mut EventProcessor, clock: &FakeClock, button: &str) -> Vec<Effect> {
    clock.advance(Duration::from_secs(1));
    p.handle_event(Event::ButtonPress {
        device: "switch".into(),
        button: button.into(),
        gesture: Gesture::Press,
        ts: clock.now(),
    })
}

fn assert_step(p: &EventProcessor, effects: &[Effect], scene_id: u8, ceiling_on: bool) {
    let messages: BTreeMap<_, Value> = effects
        .iter()
        .map(|effect| {
            (
                effect.topic(p.topology()),
                serde_json::from_str(&effect.payload_string()).unwrap(),
            )
        })
        .collect();
    let (brightness, color_temp) = match scene_id {
        3 => (160, 454),
        2 => (254, 370),
        1 => (254, 250),
        _ => panic!("invalid test scene"),
    };
    let on = json!({"state": "ON", "brightness": brightness, "color_temp": color_temp, "transition": 0.5});
    assert_eq!(messages.len(), 2);
    assert_eq!(messages["zigbee2mqtt/wall/11/set"], on);
    assert_eq!(
        messages["zigbee2mqtt/ceiling/11/set"],
        if ceiling_on {
            on
        } else {
            json!({"state": "OFF", "transition": 0.8})
        }
    );
}

#[test]
fn non_day_on_buttons_alternate_wall_and_both_for_each_scene_and_wrap() {
    for hour in [19, 23, 5] {
        let (mut p, clock) = processor(hour);
        for (scene, ceiling) in [
            (3, false),
            (3, true),
            (2, false),
            (2, true),
            (1, false),
            (1, true),
            (3, false),
        ] {
            let effects = press(&mut p, &clock, "on");
            assert_step(&p, &effects, scene, ceiling);
            assert!(
                build_full_snapshot(&p, clock.now())
                    .lights
                    .iter()
                    .all(|light| light.actual_value.is_none()),
                "commands must not fabricate device observations"
            );
        }
    }
}

#[test]
fn day_preserves_whole_group_scene_cycling() {
    let (mut p, clock) = processor(12);
    for id in [1, 2, 1] {
        let effects = press(&mut p, &clock, "on");
        assert_eq!(effects.len(), 1);
        assert_eq!(
            effects[0].topic(p.topology()),
            "zigbee2mqtt/bathroom-all/set"
        );
        assert_eq!(
            effects[0].payload_string(),
            json!({"scene_recall": id}).to_string()
        );
    }
}

#[test]
fn off_resets_the_sequence_even_before_the_device_acknowledges_off() {
    let (mut p, clock) = processor(23);
    press(&mut p, &clock, "on");
    press(&mut p, &clock, "on");
    p.handle_event(Event::GroupState {
        group: "bathroom-all".into(),
        on: true,
        ts: clock.now(),
    });
    let off = press(&mut p, &clock, "off");
    assert_eq!(off[0].topic(p.topology()), "zigbee2mqtt/bathroom-all/set");
    let effects = press(&mut p, &clock, "on");
    assert_step(&p, &effects, 3, false);
}

#[test]
fn a_new_slot_restarts_steps_without_spontaneously_changing_lights() {
    let (mut p, clock) = processor(19);
    press(&mut p, &clock, "on");
    press(&mut p, &clock, "on");
    clock.set_hour(23);
    assert!(p.handle_event(Event::Tick { ts: clock.now() }).is_empty());
    let effects = press(&mut p, &clock, "on");
    assert_step(&p, &effects, 3, false);
}

#[test]
fn entering_day_starts_the_regular_cycle_from_its_first_scene() {
    let (mut p, clock) = processor(5);
    press(&mut p, &clock, "on");
    clock.set_hour(6);
    let effects = press(&mut p, &clock, "on");
    assert_eq!(
        effects[0].topic(p.topology()),
        "zigbee2mqtt/bathroom-all/set"
    );
    assert_eq!(
        effects[0].payload_string(),
        json!({"scene_recall": 1}).to_string()
    );
}

#[test]
fn switch_steps_reject_ambiguous_endpoint_telemetry_across_rooms() {
    let mut value = configuration();
    let mut other = value["rooms"][0].clone();
    other["name"] = json!("other");
    other["group_name"] = json!("other-all");
    other["id"] = json!(2);
    other["members"] = json!(["wall/12"]);
    other["switch_steps"] = json!({});
    value["rooms"].as_array_mut().unwrap().push(other);
    let config: Config = serde_json::from_value(value).unwrap();
    let error = Topology::build(&config).unwrap_err().to_string();
    assert!(error.contains("endpoint"), "{error}");
}

#[test]
fn toggle_and_timed_tap_keep_their_off_semantics() {
    for button in ["toggle", "tap"] {
        let (mut p, clock) = processor(23);
        let effects = press(&mut p, &clock, button);
        assert_step(&p, &effects, 3, false);
        if button == "tap" {
            let effects = press(&mut p, &clock, button);
            assert_step(&p, &effects, 3, true);
        }
        clock.advance(Duration::from_secs(5));
        let off = press(&mut p, &clock, button);
        assert_eq!(off.len(), 1);
        assert_eq!(
            off[0].payload_string(),
            json!({"state": "OFF", "transition": 0.8}).to_string()
        );
    }
}

#[test]
fn invalid_switch_steps_are_rejected_by_topology() {
    for (policy, reason) in [
        (
            json!({"missing": [{"scene_id": 3, "lights": ["wall/11"]}]}),
            "slot",
        ),
        (json!({"night": []}), "empty"),
        (
            json!({"night": [{"scene_id": 99, "lights": ["wall/11"]}]}),
            "scene",
        ),
        (json!({"night": [{"scene_id": 3, "lights": []}]}), "empty"),
        (
            json!({"night": [{"scene_id": 3, "lights": ["wall/11", "wall/11"]}]}),
            "duplicate",
        ),
        (
            json!({"night": [{"scene_id": 3, "lights": ["ceiling/12"]}]}),
            "member",
        ),
    ] {
        let mut value = configuration();
        value["rooms"][0]["switch_steps"] = policy;
        let config: Config = serde_json::from_value(value).unwrap();
        let error = Topology::build(&config).unwrap_err().to_string();
        assert!(error.contains(reason), "{error}");
    }
}

#[test]
fn step_confirmation_requires_all_members_and_the_expected_settings() {
    let (mut p, clock) = processor(23);
    press(&mut p, &clock, "on");
    p.handle_event(Event::GroupState {
        group: "bathroom-all".into(),
        on: false,
        ts: clock.now(),
    });
    assert_eq!(
        phase(&p, &clock),
        "commanded",
        "transient OFF must not cancel an in-flight step"
    );
    p.handle_event(Event::GroupState {
        group: "bathroom-all".into(),
        on: true,
        ts: clock.now(),
    });
    assert_eq!(
        phase(&p, &clock),
        "commanded",
        "group ON does not confirm individual settings"
    );
    observe(&mut p, &clock, "wall", true, 254);
    observe(&mut p, &clock, "ceiling", false, 160);
    assert_eq!(phase(&p, &clock), "commanded");
    observe(&mut p, &clock, "wall", true, 160);
    assert_eq!(phase(&p, &clock), "confirmed");
    let effects = press(&mut p, &clock, "on");
    assert_step(&p, &effects, 3, true);
    observe(&mut p, &clock, "wall", true, 160);
    assert_eq!(phase(&p, &clock), "commanded");
    observe(&mut p, &clock, "ceiling", true, 160);
    assert_eq!(phase(&p, &clock), "confirmed");
    clock.advance(Duration::from_secs(1));
    observe(&mut p, &clock, "wall", false, 160);
    observe(&mut p, &clock, "ceiling", false, 160);
    let effects = press(&mut p, &clock, "on");
    assert_step(&p, &effects, 3, false);
}

#[test]
fn explicit_dashboard_scene_remains_whole_group_and_resets_switch_cursor() {
    let (mut p, clock) = processor(23);
    press(&mut p, &clock, "on");
    let effects = p.web_recall_scene("bathroom", 1, clock.now());
    assert_eq!(
        effects[0].topic(p.topology()),
        "zigbee2mqtt/bathroom-all/set"
    );
    let effects = press(&mut p, &clock, "on");
    assert_step(&p, &effects, 3, false);
}

#[test]
fn wall_only_external_off_resets_without_a_fresh_report_from_the_already_off_ceiling() {
    let (mut p, clock) = processor(23);
    press(&mut p, &clock, "on");
    observe(&mut p, &clock, "ceiling", false, 160);
    clock.advance(Duration::from_secs(1));
    observe(&mut p, &clock, "wall", true, 160);
    assert_eq!(phase(&p, &clock), "confirmed");
    clock.advance(Duration::from_secs(1));
    observe(&mut p, &clock, "wall", false, 160);
    let effects = press(&mut p, &clock, "on");
    assert_step(&p, &effects, 3, false);
}

#[test]
fn brightness_controls_only_address_selected_lights_and_keep_the_step_cursor() {
    for (button, payload) in [
        ("up", json!({"brightness_step": 25, "transition": 0.2})),
        ("hold", json!({"brightness_move": 40})),
        ("release", json!({"brightness_move": 0})),
    ] {
        let (mut p, clock) = processor(23);
        press(&mut p, &clock, "on");
        let effects = press(&mut p, &clock, button);
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].topic(p.topology()), "zigbee2mqtt/wall/11/set");
        assert_eq!(
            serde_json::from_str::<Value>(&effects[0].payload_string()).unwrap(),
            payload
        );
        observe(&mut p, &clock, "wall", true, 185);
        observe(&mut p, &clock, "ceiling", false, 160);
        assert_eq!(phase(&p, &clock), "confirmed");
        let effects = press(&mut p, &clock, "on");
        assert_step(&p, &effects, 3, true);
        assert_eq!(press(&mut p, &clock, button).len(), 2);
    }
}

#[test]
fn manual_steps_take_over_motion_owned_lights() {
    let mut value = configuration();
    value["devices"]["sensor"] = json!({"kind": "motion-sensor", "ieee_address": "0xd"});
    value["motion_rules"] = json!([{
        "name": "motion", "sensors": ["sensor"], "mode": "on-off",
        "scenes": value["rooms"][0]["scenes"],
        "targets_by_slot": {
            "day": {"group": "bathroom"},
            "evening": {"lights": ["wall/11"]},
            "night": {"lights": ["wall/11"]},
            "morning": {"lights": ["wall/11"]}
        },
        "off_cooldown_seconds": 30, "off_transition_seconds": 0.8, "max_illuminance": 15
    }]);
    let (mut p, clock) = configured_processor(value, 23);
    assert_eq!(
        p.handle_event(Event::Occupancy {
            received_at_epoch_ms: Some(clock.epoch_millis()),
            sensor: "sensor".into(),
            occupied: true,
            illuminance: Some(0),
            ts: clock.now(),
        })
        .len(),
        1
    );
    let effects = press(&mut p, &clock, "on");
    assert_step(&p, &effects, 3, false);
    assert!(
        p.handle_event(Event::Occupancy {
            received_at_epoch_ms: Some(clock.epoch_millis()),
            sensor: "sensor".into(),
            occupied: false,
            illuminance: Some(0),
            ts: clock.now(),
        })
        .is_empty(),
        "vacancy must not undo the manual step"
    );
}
