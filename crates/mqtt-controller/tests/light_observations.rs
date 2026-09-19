//! Behavioral blackbox regression tests for zone state derived from bulb reports.
#[path = "common/fixtures.rs"]
#[allow(dead_code)]
mod fixtures;

use std::sync::Arc;
use std::time::Duration;

use mqtt_controller::domain::event::Event;
use mqtt_controller::logic::EventProcessor;
use mqtt_controller::time::{Clock, FakeClock};
use mqtt_controller::topology::Topology;
use mqtt_controller::web::snapshot::build_room_snapshot;
use mqtt_controller_wire::{RoomActualValue, RoomSnapshot, RoomTargetValue};

fn processor() -> (EventProcessor, Arc<FakeClock>) {
    let config = fixtures::kitchen_config();
    let topology = Arc::new(Topology::build(&config).unwrap());
    let clock = Arc::new(FakeClock::new(12));
    (
        EventProcessor::new(topology, clock.clone(), config.defaults, None),
        clock,
    )
}

fn observe(processor: &mut EventProcessor, clock: &FakeClock, device: &str, on: bool) {
    let effects = processor.handle_event(Event::LightState {
        device: device.into(),
        on,
        brightness: None,
        color_temp: None,
        color_xy: None,
        ts: clock.now(),
    });
    assert!(effects.is_empty(), "observation must not emit commands");
}

fn room(processor: &EventProcessor, clock: &FakeClock, name: &str) -> RoomSnapshot {
    build_room_snapshot(processor, name, clock.now()).unwrap()
}

#[test]
fn unsolicited_bulb_report_updates_every_containing_zone_without_a_target() {
    let (mut p, clock) = processor();
    assert_eq!(room(&p, &clock, "kitchen-all").actual_value, None);
    observe(&mut p, &clock, "hue-l-cooker", true);
    for name in ["kitchen-cooker", "kitchen-all"] {
        let snapshot = room(&p, &clock, name);
        assert_eq!(snapshot.actual_value, Some(RoomActualValue::On), "{name}");
        assert_eq!(snapshot.target_value, None, "{name}");
        assert_eq!(snapshot.actual.unwrap().freshness, "fresh");
    }
    assert_eq!(room(&p, &clock, "kitchen-dining").actual_value, None);
}

#[test]
fn off_requires_every_member_and_partial_reports_do_not_latch_on() {
    let (mut p, clock) = processor();
    observe(&mut p, &clock, "hue-l-cooker", false);
    assert_eq!(room(&p, &clock, "kitchen-all").actual_value, None);
    assert_eq!(room(&p, &clock, "kitchen-cooker").actual_value, Some(RoomActualValue::Off));
    observe(&mut p, &clock, "hue-l-dining", true);
    assert_eq!(room(&p, &clock, "kitchen-all").actual_value, Some(RoomActualValue::On));
    observe(&mut p, &clock, "hue-l-dining", false);
    let partial = room(&p, &clock, "kitchen-all");
    assert_eq!(partial.actual_value, None);
    assert_eq!(partial.actual.unwrap().freshness, "unknown");
    observe(&mut p, &clock, "hue-l-empty", false);
    let complete = room(&p, &clock, "kitchen-all");
    assert_eq!(complete.actual_value, Some(RoomActualValue::Off));
    assert_eq!(complete.target_value, None);
}

#[test]
fn unrelated_member_reports_do_not_refresh_old_evidence() {
    let (mut p, clock) = processor();
    observe(&mut p, &clock, "hue-l-cooker", true);
    clock.advance(Duration::from_secs(30));
    observe(&mut p, &clock, "hue-l-dining", false);
    assert_eq!(room(&p, &clock, "kitchen-all").actual.unwrap().since_ago_ms, Some(30_000));
    observe(&mut p, &clock, "hue-l-cooker", false);
    clock.advance(Duration::from_secs(30));
    observe(&mut p, &clock, "hue-l-empty", false);
    let off = room(&p, &clock, "kitchen-all");
    assert_eq!(off.actual_value, Some(RoomActualValue::Off));
    assert_eq!(off.actual.unwrap().since_ago_ms, Some(30_000));
}

#[test]
fn member_observations_confirm_commands_without_fabricating_sibling_state() {
    let (mut p, clock) = processor();
    p.web_recall_scene("kitchen-all", 1, clock.now());
    observe(&mut p, &clock, "hue-l-cooker", true);
    let on = room(&p, &clock, "kitchen-all");
    assert_eq!(on.target.unwrap().phase, "confirmed");
    assert!(!p.world().lights["hue-l-dining"].actual.is_known());

    clock.advance(Duration::from_secs(1));
    p.web_set_room_off("kitchen-all", clock.now());
    observe(&mut p, &clock, "hue-l-cooker", false);
    assert_eq!(room(&p, &clock, "kitchen-all").target.unwrap().phase, "commanded");
    observe(&mut p, &clock, "hue-l-dining", false);
    assert_eq!(room(&p, &clock, "kitchen-all").target.unwrap().phase, "commanded");
    observe(&mut p, &clock, "hue-l-empty", false);
    let off = room(&p, &clock, "kitchen-all");
    assert_eq!(off.actual_value, Some(RoomActualValue::Off));
    assert_eq!(off.target.unwrap().phase, "confirmed");
}

#[test]
fn old_member_evidence_cannot_confirm_a_new_command() {
    let (mut p, clock) = processor();
    observe(&mut p, &clock, "hue-l-cooker", true);
    clock.advance(Duration::from_secs(1));
    p.web_recall_scene("kitchen-all", 1, clock.now());
    observe(&mut p, &clock, "hue-l-dining", false);
    assert_eq!(room(&p, &clock, "kitchen-all").target.unwrap().phase, "commanded");
    observe(&mut p, &clock, "hue-l-cooker", true);
    assert_eq!(room(&p, &clock, "kitchen-all").target.unwrap().phase, "confirmed");
}

#[test]
fn external_off_clears_a_confirmed_on_target_but_not_an_in_flight_command() {
    let (mut p, clock) = processor();
    p.web_recall_scene("kitchen-cooker", 1, clock.now());
    observe(&mut p, &clock, "hue-l-cooker", false);
    assert_eq!(room(&p, &clock, "kitchen-cooker").target.unwrap().phase, "commanded");
    observe(&mut p, &clock, "hue-l-cooker", true);
    clock.advance(Duration::from_secs(1));
    observe(&mut p, &clock, "hue-l-cooker", false);
    let off = room(&p, &clock, "kitchen-cooker");
    assert_eq!(off.actual_value, Some(RoomActualValue::Off));
    assert_eq!(off.target_value, Some(RoomTargetValue::Off));
    assert!(!off.physically_on);
}

#[test]
fn member_reports_update_overlapping_zones_without_a_parent_link() {
    let mut config = fixtures::kitchen_config();
    config.rooms[0].parent = None;
    config.rooms[1].parent = None;
    config.rooms[1].members.push("hue-l-cooker/11".into());
    let topology = Arc::new(Topology::build(&config).unwrap());
    let clock = Arc::new(FakeClock::new(12));
    let mut p = EventProcessor::new(topology, clock.clone(), config.defaults, None);
    observe(&mut p, &clock, "hue-l-cooker", true);
    for name in ["kitchen-cooker", "kitchen-dining", "kitchen-all"] {
        assert_eq!(room(&p, &clock, name).actual_value, Some(RoomActualValue::On), "{name}");
    }
}

#[test]
fn pre_command_off_evidence_cannot_replace_a_stale_on_target() {
    let (mut p, clock) = processor();
    for light in ["hue-l-cooker", "hue-l-dining", "hue-l-empty"] {
        observe(&mut p, &clock, light, false);
    }
    clock.advance(Duration::from_secs(1));
    p.web_recall_scene("kitchen-all", 1, clock.now());
    clock.advance(Duration::from_secs(11));
    observe(&mut p, &clock, "hue-l-cooker", false);
    assert_eq!(room(&p, &clock, "kitchen-all").target_value,
        Some(RoomTargetValue::On { scene_id: 1, cycle_idx: 0 }));
    observe(&mut p, &clock, "hue-l-dining", false);
    observe(&mut p, &clock, "hue-l-empty", false);
    assert_eq!(room(&p, &clock, "kitchen-all").target_value, Some(RoomTargetValue::Off));
}
