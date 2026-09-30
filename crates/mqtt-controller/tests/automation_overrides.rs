//! Behavioral contract for dashboard overrides of slot schedules, timed
//! actions and kill switches, against in-memory and SQLite settings stores.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mqtt_controller::config::Config;
use mqtt_controller::config::heating::TemperatureSchedule;
use mqtt_controller::config::scenes::{Slot, SlotName};
use mqtt_controller::config::switch_model::Gesture;
use mqtt_controller::config::time_expr::TimeExpr;
use mqtt_controller::domain::{Effect, event::Event};
use mqtt_controller::logic::EventProcessor;
use mqtt_controller::settings::{
    ControlSettings, KillSwitchOverride, PlugScheduleOverride, PlugSchedulePlan, PlugTimedAction,
    RoomScheduleOverride, RoomSchedulePlan, SettingsRepository, SqliteSettings, ValveBoost,
    change_motion_schedule, change_plug_schedule, change_room_schedule, change_timed_action_time,
};
use mqtt_controller::time::{Clock, FakeClock};
use mqtt_controller::topology::Topology;
use mqtt_controller::web::snapshot::{build_plug_snapshot, build_room_snapshot};
use mqtt_controller_wire::{KillSwitchPlan, PlugAction, PlugTimedActionPlan, SlotPlan, SwitchStepPlan};
use serde_json::{Value, json};

const ROOM: &str = "bathroom";
const RULE: &str = "bathroom-motion";
const PLUG: &str = "printer";

fn config() -> Config {
    let scenes = json!({
        "scenes": [
            {"id": 1, "name": "bright", "brightness": 254, "color_temp": 250, "transition": 0.5},
            {"id": 2, "name": "warm", "brightness": 160, "color_temp": 454, "transition": 0.5}
        ],
        "slots": {
            "day": {"from": "06:00", "to": "18:00", "scene_ids": [1, 2]},
            "night": {"from": "18:00", "to": "06:00", "scene_ids": [2, 1]}
        }
    });
    serde_json::from_value(json!({
        "devices": {
            "wall": {"kind": "light", "ieee_address": "0xa"},
            "ceiling": {"kind": "light", "ieee_address": "0xb"},
            "sensor": {"kind": "motion-sensor", "ieee_address": "0xc", "occupancy_timeout_seconds": 60},
            "switch": {"kind": "switch", "ieee_address": "0xd", "model": "test"},
            "printer": {"kind": "plug", "ieee_address": "0xe", "variant": "sonoff-power", "capabilities": ["power"]}
        },
        "switch_models": {"test": {"buttons": ["toggle"], "z2m_action_map": {}}},
        "rooms": [{
            "name": ROOM, "room": ROOM, "group_name": "bathroom-all", "id": 1,
            "members": ["wall/11", "ceiling/11"], "off_transition_seconds": 0.8, "scenes": scenes
        }],
        "motion_rules": [{
            "name": RULE, "sensors": ["sensor"], "mode": "on-off", "scenes": scenes,
            "off_transition_seconds": 0.8, "off_cooldown_seconds": 0,
            "targets_by_slot": {"day": {"group": ROOM}, "night": {"lights": ["wall/11"]}}
        }],
        "bindings": [
            {"name": "toggle", "trigger": {"kind": "button", "device": "switch", "button": "toggle", "gesture": "press"},
             "effect": {"kind": "scene_toggle", "room": ROOM}},
            {"name": "night-off", "trigger": {"kind": "at", "time": "23:00"}, "effect": {"kind": "turn_off_room", "room": ROOM}},
            {"name": "printer-on", "trigger": {"kind": "at", "time": "07:00"}, "effect": {"kind": "turn_on", "target": PLUG}},
            {"name": "printer-idle", "trigger": {"kind": "power_below", "device": PLUG, "watts": 5.0, "for_seconds": 600},
             "effect": {"kind": "turn_off", "target": PLUG}}
        ]
    }))
    .expect("override fixture must deserialize")
}

fn processor(hour: u8) -> (EventProcessor, Arc<Topology>, Arc<FakeClock>) {
    let cfg = config();
    let topology = Arc::new(Topology::build(&cfg).unwrap());
    let clock = Arc::new(FakeClock::new(hour));
    (EventProcessor::new(topology.clone(), clock.clone(), cfg.defaults, None), topology, clock)
}

fn published(effects: &[Effect], topology: &Topology) -> Vec<(String, Value)> {
    effects
        .iter()
        .map(|effect| (effect.topic(topology), serde_json::from_str(&effect.payload_string()).unwrap()))
        .collect()
}

fn toggle(p: &mut EventProcessor, clock: &FakeClock) -> Vec<Effect> {
    p.handle_event(Event::ButtonPress { device: "switch".into(), button: "toggle".into(), gesture: Gesture::Press, ts: clock.now() })
}

fn occupancy(clock: &FakeClock, occupied: bool) -> Event {
    Event::Occupancy {
        sensor: "sensor".into(), occupied, illuminance: Some(0),
        received_at_epoch_ms: Some(clock.epoch_millis()), ts: clock.now(),
    }
}

fn plan(slots: &[(&str, &str, &str, &[u8])]) -> Vec<SlotPlan> {
    slots
        .iter()
        .map(|(name, from, to, scene_ids)| SlotPlan {
            name: (*name).into(), from: (*from).into(), to: (*to).into(), scene_ids: scene_ids.to_vec(),
        })
        .collect()
}

fn room_plan(slots: &[SlotPlan], steps: &[(&str, &[(u8, &[&str])])]) -> RoomSchedulePlan {
    RoomSchedulePlan {
        slots: slots.to_vec(),
        switch_steps: steps
            .iter()
            .map(|(slot, steps)| ((*slot).to_string(), steps.iter().map(|(scene_id, lights)| SwitchStepPlan {
                scene_id: *scene_id, lights: lights.iter().map(|light| (*light).to_string()).collect(),
            }).collect()))
            .collect(),
    }
}

fn room_override(plan: &RoomSchedulePlan) -> RoomScheduleOverride {
    RoomScheduleOverride {
        slots: slots(&plan.slots),
        switch_steps: plan.switch_steps.iter().map(|(slot, steps)| (slot.clone(), steps.iter().map(|step| {
            mqtt_controller::config::SwitchStep { scene_id: step.scene_id, lights: step.lights.clone() }
        }).collect())).collect(),
    }
}

fn plug_plan(actions: &[(&str, PlugAction)], kill_switch: Option<(f64, u64)>) -> PlugSchedulePlan {
    PlugSchedulePlan {
        timed_actions: actions.iter().map(|(time, action)| PlugTimedActionPlan { time: (*time).into(), action: *action }).collect(),
        kill_switch: kill_switch.map(|(threshold_watts, holdoff_secs)| KillSwitchPlan { threshold_watts, holdoff_secs }),
    }
}

fn plug_override(plan: &PlugSchedulePlan) -> PlugScheduleOverride {
    PlugScheduleOverride {
        timed_actions: plan.timed_actions.iter().map(|action| PlugTimedAction { time: action.time.parse().unwrap(), action: action.action }).collect(),
        kill_switch: plan.kill_switch.as_ref().map(|plan| KillSwitchOverride { threshold_watts: plan.threshold_watts, holdoff_secs: plan.holdoff_secs }),
    }
}

fn slots(plans: &[SlotPlan]) -> BTreeMap<SlotName, Slot> {
    plans
        .iter()
        .map(|plan| (plan.name.clone(), Slot {
            from: plan.from.parse().unwrap(), to: plan.to.parse().unwrap(), scene_ids: plan.scene_ids.clone(),
        }))
        .collect()
}

/// Scene recalled by the first ON toggle from OFF at hour 12.
fn toggled_scene(p: &mut EventProcessor, topology: &Topology, clock: &FakeClock) -> Value {
    let effects = published(&toggle(p, clock), topology);
    assert_eq!(effects.len(), 1, "{effects:?}");
    assert_eq!(effects[0].0, "zigbee2mqtt/bathroom-all/set");
    effects[0].1["scene_recall"].clone()
}

async fn stored_toggled_scene(repository: &impl SettingsRepository) -> Value {
    let (mut p, topology, clock) = processor(12);
    p.restore_settings(repository.load().await.unwrap());
    toggled_scene(&mut p, &topology, &clock)
}

/// Motion target at hour 12 on a fresh processor with the stored settings;
/// earlier manual scenes would otherwise keep motion away from the lights.
async fn motion_topic(repository: &impl SettingsRepository) -> String {
    let (mut p, topology, clock) = processor(12);
    p.restore_settings(repository.load().await.unwrap());
    let on = published(&p.handle_event(occupancy(&clock, true)), &topology);
    assert_eq!(on.len(), 1, "{on:?}");
    on[0].0.clone()
}

/// Effects of the `At` tick at 12:30.
fn half_past_noon_tick(p: &mut EventProcessor, topology: &Topology, clock: &FakeClock) -> Vec<(String, Value)> {
    clock.set_minute(29);
    p.handle_event(Event::Tick { ts: clock.now() });
    clock.set_minute(30);
    let effects = published(&p.handle_event(Event::Tick { ts: clock.now() }), topology);
    clock.set_minute(0);
    effects
}

/// Plug power effects after running below 20 W for 90 seconds.
fn idle_printer(p: &mut EventProcessor, topology: &Topology, clock: &FakeClock) -> Vec<(String, Value)> {
    p.handle_event(Event::PlugState { device: PLUG.into(), on: false, power: Some(0.0), ts: clock.now() });
    p.handle_event(Event::PlugState { device: PLUG.into(), on: true, power: Some(100.0), ts: clock.now() });
    p.handle_event(Event::PlugState { device: PLUG.into(), on: true, power: Some(20.0), ts: clock.now() });
    clock.advance(Duration::from_secs(90));
    published(&p.handle_event(Event::Tick { ts: clock.now() }), topology)
        .into_iter()
        .filter(|(topic, _)| topic.contains(PLUG))
        .collect()
}

async fn override_contract(repository: &impl SettingsRepository) {
    let (mut p, topology, clock) = processor(12);
    assert_eq!(stored_toggled_scene(repository).await, json!(1));
    assert_eq!(motion_topic(repository).await, "zigbee2mqtt/bathroom-all/set");
    assert!(half_past_noon_tick(&mut p, &topology, &clock).is_empty());
    assert!(idle_printer(&mut p, &topology, &clock).is_empty());

    let day_night = plan(&[("day", "06:00", "18:00", &[2, 1]), ("night", "18:00", "06:00", &[2])]);
    let whole_group = room_plan(&day_night, &[]);
    for (invalid, reason) in [
        (room_plan(&[], &[]), "at least one slot"),
        (room_plan(&plan(&[(" ", "00:00", "24:00", &[1])]), &[]), "blank"),
        (room_plan(&plan(&[("day", "06:00", "19:00", &[1]), ("night", "18:00", "06:00", &[2])]), &[]), "multiple slots"),
        (room_plan(&plan(&[("day", "06:00", "17:00", &[1]), ("night", "18:00", "06:00", &[2])]), &[]), "uncovered"),
        (room_plan(&plan(&[("day", "06:00", "18:00", &[9]), ("night", "18:00", "06:00", &[2])]), &[]), "scene id 9"),
        (room_plan(&plan(&[("day", "sunrise", "sunset", &[1]), ("night", "sunset", "sunrise", &[2])]), &[]), "location"),
        (room_plan(&plan(&[("day", "6am", "18:00", &[1]), ("night", "18:00", "06:00", &[2])]), &[]), "invalid"),
        (room_plan(&day_night, &[("dusk", &[(1, &["wall/11"])])]), "unknown slot"),
        (room_plan(&day_night, &[("night", &[])]), "empty step sequence"),
        (room_plan(&day_night, &[("night", &[(9, &["wall/11"])])]), "unknown scene 9"),
        (room_plan(&day_night, &[("night", &[(1, &["attic/11"])])]), "not a room member"),
        (room_plan(&day_night, &[("night", &[(1, &["wall/11", "wall/11"])])]), "duplicate lights"),
    ] {
        let error = change_room_schedule(&mut p, repository, ROOM, Some(invalid)).await.unwrap_err();
        assert!(error.contains(reason), "{error}");
    }
    assert!(change_room_schedule(&mut p, repository, "attic", Some(whole_group.clone())).await.is_err());
    change_room_schedule(&mut p, repository, ROOM, Some(whole_group.clone())).await.unwrap();
    assert_eq!(repository.load().await.unwrap().room_schedule_overrides[ROOM], room_override(&whole_group));
    assert_eq!(stored_toggled_scene(repository).await, json!(2));
    let room = build_room_snapshot(&p, ROOM, clock.now()).unwrap();
    assert!(room.schedule.overridden);
    assert_eq!(room.schedule.slots, whole_group.slots);
    assert_eq!(room.schedule.available_scenes.iter().map(|scene| (scene.id, scene.name.as_str())).collect::<Vec<_>>(), vec![(1, "bright"), (2, "warm")]);
    assert_eq!(room.members, vec!["wall/11", "ceiling/11"]);
    assert!(room.switch_steps.is_empty());
    assert_eq!(room.scene_ids, vec![2, 1]);
    assert!(!room.motion_rules[0].schedule.overridden);

    // Slots may be added and renamed; switch steps stagger the new slot's lights.
    let staggered = room_plan(
        &plan(&[("morning", "06:00", "12:00", &[1]), ("afternoon", "12:00", "18:00", &[2, 1]), ("night", "18:00", "06:00", &[2])]),
        &[("afternoon", &[(2, &["wall/11"]), (2, &["wall/11", "ceiling/11"])])],
    );
    change_room_schedule(&mut p, repository, ROOM, Some(staggered.clone())).await.unwrap();
    let room = build_room_snapshot(&p, ROOM, clock.now()).unwrap();
    assert_eq!(room.active_slot.as_deref(), Some("afternoon"));
    assert_eq!(room.switch_steps, staggered.switch_steps);
    let stepped = published(&toggle(&mut p, &clock), &topology);
    let topics: Vec<&str> = stepped.iter().map(|(topic, _)| topic.as_str()).collect();
    assert_eq!(topics, vec!["zigbee2mqtt/wall/11/set", "zigbee2mqtt/ceiling/11/set"], "{stepped:?}");
    assert_eq!(stepped[0].1, json!({"state": "ON", "brightness": 160, "color_temp": 454, "transition": 0.5}));
    assert_eq!(stepped[1].1, json!({"state": "OFF", "transition": 0.8}));
    change_room_schedule(&mut p, repository, ROOM, Some(whole_group.clone())).await.unwrap();
    let off = published(&toggle(&mut p, &clock), &topology);
    assert!(off.iter().any(|(topic, payload)| topic == "zigbee2mqtt/bathroom-all/set" && payload["state"] == "OFF"), "{off:?}");
    assert_eq!(toggled_scene(&mut p, &topology, &clock), json!(2), "the step selection is forgotten with its steps");

    let motion_plan = plan(&[("day", "06:00", "11:00", &[1]), ("night", "11:00", "06:00", &[2])]);
    let empty = plan(&[("day", "06:00", "11:00", &[]), ("night", "11:00", "06:00", &[2])]);
    let renamed = plan(&[("day", "06:00", "11:00", &[1]), ("dusk", "11:00", "06:00", &[2])]);
    assert!(change_motion_schedule(&mut p, repository, RULE, Some(empty)).await.unwrap_err().contains("no scenes"));
    assert!(change_motion_schedule(&mut p, repository, RULE, Some(renamed)).await.unwrap_err().contains("configured slots"));
    change_motion_schedule(&mut p, repository, RULE, Some(motion_plan.clone())).await.unwrap();
    assert_eq!(motion_topic(repository).await, "zigbee2mqtt/wall/11/set");
    let rule = &build_room_snapshot(&p, ROOM, clock.now()).unwrap().motion_rules[0];
    assert!(rule.schedule.overridden);
    assert_eq!(rule.active_slot.as_deref(), Some("night"));

    for (binding, time, reason) in [
        ("night-off", "24:00", "cannot run"),
        ("night-off", "sunset", "location"),
        ("night-off", "noon", "invalid"),
        ("toggle", "12:30", "not a timed action"),
        ("printer-on", "12:30", "edit the plug schedule"),
        ("missing", "12:30", "Unknown binding"),
    ] {
        let error = change_timed_action_time(&mut p, repository, binding, Some(time)).await.unwrap_err();
        assert!(error.contains(reason), "{error}");
    }
    change_timed_action_time(&mut p, repository, "night-off", Some("12:30")).await.unwrap();
    assert_eq!(repository.load().await.unwrap().timed_action_overrides["night-off"], "12:30".parse::<TimeExpr>().unwrap());
    p.set_zone_actual(ROOM, true, clock.now());
    let fired = half_past_noon_tick(&mut p, &topology, &clock);
    assert!(fired.contains(&("zigbee2mqtt/bathroom-all/set".into(), json!({"state": "OFF", "transition": 0.8}))), "{fired:?}");
    assert!(!fired.iter().any(|(topic, _)| topic == "zigbee2mqtt/printer/set"), "{fired:?}");
    let room = build_room_snapshot(&p, ROOM, clock.now()).unwrap();
    assert_eq!(room.timed_actions.len(), 1);
    assert_eq!((room.timed_actions[0].time.as_str(), room.timed_actions[0].overridden), ("12:30", true));
    let plug = build_plug_snapshot(&p, PLUG, clock.now()).unwrap();
    assert!(!plug.schedule.overridden && plug.schedule.power_metered);
    assert_eq!(plug.schedule.timed_actions, vec![PlugTimedActionPlan { time: "07:00".into(), action: PlugAction::On }]);
    assert_eq!(plug.schedule.kill_switch, Some(KillSwitchPlan { threshold_watts: 5.0, holdoff_secs: 600 }));

    for (device, invalid, reason) in [
        (PLUG, plug_plan(&[("24:00", PlugAction::On)], None), "cannot run"),
        (PLUG, plug_plan(&[("sunset", PlugAction::On)], None), "location"),
        (PLUG, plug_plan(&[("noon", PlugAction::On)], None), "invalid"),
        (PLUG, plug_plan(&[], Some((0.0, 60))), "positive"),
        (PLUG, plug_plan(&[], Some((f64::NAN, 60))), "positive"),
        (PLUG, plug_plan(&[], Some((50.0, 0))), "at least one second"),
        ("wall", plug_plan(&[], None), "Unknown plug"),
        ("missing", plug_plan(&[], None), "Unknown plug"),
    ] {
        let error = change_plug_schedule(&mut p, repository, device, Some(invalid)).await.unwrap_err();
        assert!(error.contains(reason), "{error}");
    }
    assert!(change_plug_schedule(&mut p, repository, "wall", None).await.unwrap_err().contains("Unknown plug"));
    let scheduled = plug_plan(&[("12:30", PlugAction::On), ("13:00", PlugAction::Off)], Some((50.0, 60)));
    change_plug_schedule(&mut p, repository, PLUG, Some(scheduled.clone())).await.unwrap();
    assert_eq!(repository.load().await.unwrap().plug_schedule_overrides[PLUG], plug_override(&scheduled));
    let plug = build_plug_snapshot(&p, PLUG, clock.now()).unwrap();
    assert!(plug.schedule.overridden);
    assert_eq!(plug.schedule.timed_actions, scheduled.timed_actions);
    assert_eq!(plug.schedule.kill_switch, scheduled.kill_switch);
    assert_eq!((plug.kill_switch_rules[0].threshold_watts, plug.kill_switch_rules[0].holdoff_secs), (50.0, 60));
    assert!(plug.kill_switch_rules[0].overridden);
    let fired = half_past_noon_tick(&mut p, &topology, &clock);
    assert!(fired.iter().any(|(topic, payload)| topic == "zigbee2mqtt/printer/set" && payload["state"] == "ON"), "{fired:?}");
    assert_eq!(idle_printer(&mut p, &topology, &clock), vec![("zigbee2mqtt/printer/set".into(), json!({"state": "OFF"}))]);

    // A schedule without a kill switch also removes the deployed one.
    change_plug_schedule(&mut p, repository, PLUG, Some(plug_plan(&[("12:30", PlugAction::On)], None))).await.unwrap();
    let plug = build_plug_snapshot(&p, PLUG, clock.now()).unwrap();
    assert!(plug.schedule.kill_switch.is_none() && plug.kill_switch_rules.is_empty());
    assert!(idle_printer(&mut p, &topology, &clock).is_empty());
    change_plug_schedule(&mut p, repository, PLUG, Some(scheduled.clone())).await.unwrap();

    let (mut restored, topology, clock) = processor(12);
    restored.restore_settings(repository.load().await.unwrap());
    assert_eq!(stored_toggled_scene(repository).await, json!(2));
    assert_eq!(motion_topic(repository).await, "zigbee2mqtt/wall/11/set");
    let fired = half_past_noon_tick(&mut restored, &topology, &clock);
    assert!(fired.iter().any(|(topic, payload)| topic == "zigbee2mqtt/printer/set" && payload["state"] == "ON"), "{fired:?}");
    assert_eq!(idle_printer(&mut restored, &topology, &clock).len(), 1);

    change_room_schedule(&mut restored, repository, ROOM, None).await.unwrap();
    change_motion_schedule(&mut restored, repository, RULE, None).await.unwrap();
    change_timed_action_time(&mut restored, repository, "night-off", None).await.unwrap();
    change_plug_schedule(&mut restored, repository, PLUG, None).await.unwrap();
    assert_eq!(repository.load().await.unwrap(), ControlSettings::default());
    assert_eq!(stored_toggled_scene(repository).await, json!(1));
    assert_eq!(motion_topic(repository).await, "zigbee2mqtt/bathroom-all/set");
    assert!(half_past_noon_tick(&mut restored, &topology, &clock).is_empty());
    assert!(idle_printer(&mut restored, &topology, &clock).is_empty());
    let room = build_room_snapshot(&restored, ROOM, clock.now()).unwrap();
    assert!(!room.schedule.overridden && !room.motion_rules[0].schedule.overridden && !room.timed_actions[0].overridden);
    let plug = build_plug_snapshot(&restored, PLUG, clock.now()).unwrap();
    assert!(!plug.kill_switch_rules[0].overridden && !plug.schedule.overridden);
    assert_eq!(plug.schedule.timed_actions[0].time, "07:00");
}

#[derive(Default)]
struct MemorySettings(Mutex<ControlSettings>);

impl SettingsRepository for MemorySettings {
    async fn set_valve_boost(&self, _: &str, _: Option<&ValveBoost>) -> anyhow::Result<()> { unreachable!() }
    async fn set_valve_schedule(&self, _: &str, _: Option<&TemperatureSchedule>) -> anyhow::Result<()> { unreachable!() }
    async fn set_motion_enabled(&self, _: &str, _: bool) -> anyhow::Result<()> { unreachable!() }
    async fn set_heat_demand_enabled(&self, _: &str, _: bool) -> anyhow::Result<()> { unreachable!() }
    async fn set_room_schedule(&self, room: &str, value: Option<&RoomScheduleOverride>) -> anyhow::Result<()> {
        let mut settings = self.0.lock().unwrap();
        match value {
            Some(value) => settings.room_schedule_overrides.insert(room.into(), value.clone()),
            None => settings.room_schedule_overrides.remove(room),
        };
        Ok(())
    }
    async fn set_motion_schedule(&self, rule: &str, slots: Option<&BTreeMap<SlotName, Slot>>) -> anyhow::Result<()> {
        let mut settings = self.0.lock().unwrap();
        match slots {
            Some(slots) => settings.motion_schedule_overrides.insert(rule.into(), slots.clone()),
            None => settings.motion_schedule_overrides.remove(rule),
        };
        Ok(())
    }
    async fn set_timed_action_time(&self, binding: &str, time: Option<&TimeExpr>) -> anyhow::Result<()> {
        let mut settings = self.0.lock().unwrap();
        match time {
            Some(time) => settings.timed_action_overrides.insert(binding.into(), time.clone()),
            None => settings.timed_action_overrides.remove(binding),
        };
        Ok(())
    }
    async fn set_plug_schedule(&self, device: &str, value: Option<&PlugScheduleOverride>) -> anyhow::Result<()> {
        let mut settings = self.0.lock().unwrap();
        match value {
            Some(value) => settings.plug_schedule_overrides.insert(device.into(), value.clone()),
            None => settings.plug_schedule_overrides.remove(device),
        };
        Ok(())
    }
    async fn load(&self) -> anyhow::Result<ControlSettings> {
        Ok(self.0.lock().unwrap().clone())
    }
}

#[tokio::test]
async fn override_contract_with_memory_store() {
    override_contract(&MemorySettings::default()).await;
}

#[tokio::test]
async fn override_contract_with_sqlite_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.db");
    let repository = SqliteSettings::open(&path).await.unwrap();
    override_contract(&repository).await;
    let saved = room_plan(
        &plan(&[("day", "sunrise+01:00", "max(sunset, 20:00)", &[2]), ("night", "max(sunset, 20:00)", "sunrise+01:00", &[1])]),
        &[("night", &[(1, &["wall/11"])])],
    );
    repository.set_room_schedule(ROOM, Some(&room_override(&saved))).await.unwrap();
    repository.set_motion_schedule(RULE, Some(&slots(&saved.slots))).await.unwrap();
    repository.set_timed_action_time("night-off", Some(&"sunset-00:30".parse().unwrap())).await.unwrap();
    repository.set_plug_schedule(PLUG, Some(&plug_override(&plug_plan(&[("sunset-00:30", PlugAction::Toggle)], Some((2.5, 30)))))).await.unwrap();
    let expected = repository.load().await.unwrap();
    drop(repository);
    assert_eq!(SqliteSettings::open(&path).await.unwrap().load().await.unwrap(), expected);
}

struct UnwritableSettings;

impl SettingsRepository for UnwritableSettings {
    async fn set_valve_boost(&self, _: &str, _: Option<&ValveBoost>) -> anyhow::Result<()> { anyhow::bail!("read-only database") }
    async fn set_valve_schedule(&self, _: &str, _: Option<&TemperatureSchedule>) -> anyhow::Result<()> { anyhow::bail!("read-only database") }
    async fn set_motion_enabled(&self, _: &str, _: bool) -> anyhow::Result<()> { anyhow::bail!("read-only database") }
    async fn set_heat_demand_enabled(&self, _: &str, _: bool) -> anyhow::Result<()> { anyhow::bail!("read-only database") }
    async fn set_room_schedule(&self, _: &str, _: Option<&RoomScheduleOverride>) -> anyhow::Result<()> { anyhow::bail!("read-only database") }
    async fn set_motion_schedule(&self, _: &str, _: Option<&BTreeMap<SlotName, Slot>>) -> anyhow::Result<()> { anyhow::bail!("read-only database") }
    async fn set_timed_action_time(&self, _: &str, _: Option<&TimeExpr>) -> anyhow::Result<()> { anyhow::bail!("read-only database") }
    async fn set_plug_schedule(&self, _: &str, _: Option<&PlugScheduleOverride>) -> anyhow::Result<()> { anyhow::bail!("read-only database") }
    async fn load(&self) -> anyhow::Result<ControlSettings> { Ok(ControlSettings::default()) }
}

#[tokio::test]
async fn failed_save_keeps_deployed_behavior() {
    let (mut p, topology, clock) = processor(12);
    let day_night = plan(&[("day", "06:00", "18:00", &[2, 1]), ("night", "18:00", "06:00", &[2])]);
    for error in [
        change_room_schedule(&mut p, &UnwritableSettings, ROOM, Some(room_plan(&day_night, &[]))).await,
        change_motion_schedule(&mut p, &UnwritableSettings, RULE, Some(day_night.clone())).await,
        change_timed_action_time(&mut p, &UnwritableSettings, "night-off", Some("12:30")).await,
        change_plug_schedule(&mut p, &UnwritableSettings, PLUG, Some(plug_plan(&[], Some((50.0, 60))))).await,
    ] {
        assert!(error.unwrap_err().contains("read-only database"));
    }
    assert_eq!(toggled_scene(&mut p, &topology, &clock), json!(1));
    p.set_zone_actual(ROOM, true, clock.now());
    assert!(half_past_noon_tick(&mut p, &topology, &clock).is_empty());
    assert!(idle_printer(&mut p, &topology, &clock).is_empty());
}

#[test]
fn restore_ignores_overrides_that_no_longer_fit_the_deployment() {
    let (mut p, topology, clock) = processor(12);
    let mut settings = ControlSettings::default();
    settings.room_schedule_overrides.insert(ROOM.into(), room_override(&room_plan(
        &plan(&[("day", "00:00", "24:00", &[2])]), &[("day", &[(2, &["attic/11"])])],
    )));
    settings.motion_schedule_overrides.insert("removed-rule".into(), BTreeMap::new());
    settings.motion_schedule_overrides.insert(RULE.into(), slots(&plan(&[("day", "00:00", "24:00", &[2])])));
    settings.timed_action_overrides.insert("toggle".into(), "12:30".parse().unwrap());
    settings.timed_action_overrides.insert("printer-on".into(), "12:30".parse().unwrap());
    settings.plug_schedule_overrides.insert("removed".into(), plug_override(&plug_plan(&[], Some((50.0, 60)))));
    p.restore_settings(settings);
    assert_eq!(toggled_scene(&mut p, &topology, &clock), json!(1));
    let room = build_room_snapshot(&p, ROOM, clock.now()).unwrap();
    assert!(!room.schedule.overridden && !room.motion_rules[0].schedule.overridden);
}

#[tokio::test]
async fn legacy_slot_override_table_is_migrated() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.db");
    let database = turso::Builder::new_local(path.to_str().unwrap()).build().await.unwrap();
    let connection = database.connect().unwrap();
    connection.execute(
        "CREATE TABLE scene_schedule_overrides (owner_kind TEXT NOT NULL, owner TEXT NOT NULL, slots_json TEXT NOT NULL, PRIMARY KEY(owner_kind, owner))", (),
    ).await.unwrap();
    let day = serde_json::to_string(&slots(&plan(&[("day", "00:00", "24:00", &[2])]))).unwrap();
    for (kind, owner) in [("room", ROOM), ("motion_rule", RULE)] {
        connection.execute("INSERT INTO scene_schedule_overrides VALUES (?, ?, ?)", turso::params![kind, owner, day.clone()]).await.unwrap();
    }
    connection.execute("CREATE TABLE kill_switch_overrides (binding TEXT PRIMARY KEY NOT NULL, threshold_watts REAL NOT NULL, holdoff_secs INTEGER NOT NULL)", ()).await.unwrap();
    connection.execute("INSERT INTO kill_switch_overrides VALUES ('printer-idle', 2.5, 30)", ()).await.unwrap();
    drop(connection);
    drop(database);
    let loaded = SqliteSettings::open(&path).await.unwrap().load().await.unwrap();
    assert!(loaded.room_schedule_overrides.is_empty() && loaded.plug_schedule_overrides.is_empty());
    assert_eq!(loaded.motion_schedule_overrides[RULE], slots(&plan(&[("day", "00:00", "24:00", &[2])])));
    let reopened = SqliteSettings::open(&path).await.unwrap().load().await.unwrap();
    assert_eq!(reopened, loaded);
}
