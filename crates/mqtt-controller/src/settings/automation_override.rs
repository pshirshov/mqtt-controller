//! Dashboard overrides of deployed light and plug automation: slot
//! schedules, switch steps, timed binding times and kill-switch parameters.

use std::collections::BTreeMap;

use mqtt_controller_wire::{KillSwitchPlan, PlugAction, PlugTimedActionPlan, SlotPlan, SwitchStepPlan};
use serde::{Deserialize, Serialize};

use crate::config::room::SwitchStep;
use crate::config::scenes::{Slot, SlotName};
use crate::config::time_expr::TimeExpr;
use crate::logic::EventProcessor;

use super::SettingsRepository;

/// Owner of a slot schedule. Rooms and motion rules have separate namespaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleOwner<'a> {
    Room(&'a str),
    MotionRule(&'a str),
}

/// Replacement of a light group's slot schedule and switch steps. The
/// switch-step map is complete: slots absent from it cycle the whole group.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoomScheduleOverride {
    pub slots: BTreeMap<SlotName, Slot>,
    pub switch_steps: BTreeMap<SlotName, Vec<SwitchStep>>,
}

/// Wire form of [`RoomScheduleOverride`].
#[derive(Debug, Clone, PartialEq)]
pub struct RoomSchedulePlan {
    pub slots: Vec<SlotPlan>,
    pub switch_steps: BTreeMap<String, Vec<SwitchStepPlan>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KillSwitchOverride {
    pub threshold_watts: f64,
    pub holdoff_secs: u64,
}

/// One daily action of a dashboard-defined plug schedule.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlugTimedAction {
    pub time: TimeExpr,
    pub action: PlugAction,
}

/// Replacement of a plug's daily actions and kill switch. While present it
/// supersedes every deployed binding that times or power-guards the plug.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlugScheduleOverride {
    pub timed_actions: Vec<PlugTimedAction>,
    pub kill_switch: Option<KillSwitchOverride>,
}

/// Wire form of [`PlugScheduleOverride`].
#[derive(Debug, Clone, PartialEq)]
pub struct PlugSchedulePlan {
    pub timed_actions: Vec<PlugTimedActionPlan>,
    pub kill_switch: Option<KillSwitchPlan>,
}

fn parse_plug_plan(plan: PlugSchedulePlan) -> Result<PlugScheduleOverride, String> {
    Ok(PlugScheduleOverride {
        timed_actions: plan
            .timed_actions
            .into_iter()
            .map(|action| Ok(PlugTimedAction {
                time: action.time.parse::<TimeExpr>().map_err(|error| error.to_string())?,
                action: action.action,
            }))
            .collect::<Result<_, String>>()?,
        kill_switch: plan.kill_switch.map(|plan| KillSwitchOverride {
            threshold_watts: plan.threshold_watts,
            holdoff_secs: plan.holdoff_secs,
        }),
    })
}

impl KillSwitchOverride {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !self.threshold_watts.is_finite() || self.threshold_watts <= 0.0 {
            return Err("Kill-switch threshold must be a positive number of watts".into());
        }
        if self.holdoff_secs == 0 {
            return Err("Kill-switch holdoff must be at least one second".into());
        }
        Ok(())
    }
}

pub fn slot_plans(slots: &BTreeMap<SlotName, Slot>) -> Vec<SlotPlan> {
    slots
        .iter()
        .map(|(name, slot)| SlotPlan {
            name: name.clone(),
            from: slot.from.to_string(),
            to: slot.to.to_string(),
            scene_ids: slot.scene_ids.clone(),
        })
        .collect()
}

fn parse_slots(plans: Vec<SlotPlan>) -> Result<BTreeMap<SlotName, Slot>, String> {
    let mut slots = BTreeMap::new();
    for plan in plans {
        let parse = |value: &str| {
            value
                .parse::<TimeExpr>()
                .map_err(|error| format!("Slot {}: {error}", plan.name))
        };
        let slot = Slot {
            from: parse(&plan.from)?,
            to: parse(&plan.to)?,
            scene_ids: plan.scene_ids,
        };
        if slots.insert(plan.name.clone(), slot).is_some() {
            return Err(format!("Slot {} is listed more than once", plan.name));
        }
    }
    Ok(slots)
}

fn parse_room_plan(plan: RoomSchedulePlan) -> Result<RoomScheduleOverride, String> {
    Ok(RoomScheduleOverride {
        slots: parse_slots(plan.slots)?,
        switch_steps: plan
            .switch_steps
            .into_iter()
            .map(|(slot, steps)| {
                let steps = steps
                    .into_iter()
                    .map(|step| SwitchStep { scene_id: step.scene_id, lights: step.lights })
                    .collect();
                (slot, steps)
            })
            .collect(),
    })
}

pub async fn change_room_schedule(
    processor: &mut EventProcessor,
    repository: &impl SettingsRepository,
    room: &str,
    plan: Option<RoomSchedulePlan>,
) -> Result<(), String> {
    processor.validate_schedule_owner(ScheduleOwner::Room(room))?;
    let value = plan.map(parse_room_plan).transpose()?;
    let resolved = match &value {
        Some(value) => Some(processor.validate_room_override(room, value)?),
        None => None,
    };
    repository
        .set_room_schedule(room, value.as_ref())
        .await
        .map_err(|error| format!("Could not save schedule for {room}: {error}"))?;
    processor.apply_room_override(room, value.zip(resolved));
    Ok(())
}

pub async fn change_motion_schedule(
    processor: &mut EventProcessor,
    repository: &impl SettingsRepository,
    rule: &str,
    plans: Option<Vec<SlotPlan>>,
) -> Result<(), String> {
    processor.validate_schedule_owner(ScheduleOwner::MotionRule(rule))?;
    let slots = plans.map(parse_slots).transpose()?;
    if let Some(slots) = &slots {
        processor.validate_motion_override(rule, slots)?;
    }
    repository
        .set_motion_schedule(rule, slots.as_ref())
        .await
        .map_err(|error| format!("Could not save schedule for {rule}: {error}"))?;
    processor.apply_motion_override(rule, slots);
    Ok(())
}

pub async fn change_timed_action_time(
    processor: &mut EventProcessor,
    repository: &impl SettingsRepository,
    binding: &str,
    time: Option<&str>,
) -> Result<(), String> {
    processor.validate_timed_action(binding)?;
    let time = time
        .map(|time| time.parse::<TimeExpr>().map_err(|error| error.to_string()))
        .transpose()?;
    if let Some(time) = &time {
        processor.validate_timed_action_time(time)?;
    }
    repository
        .set_timed_action_time(binding, time.as_ref())
        .await
        .map_err(|error| format!("Could not save time for {binding}: {error}"))?;
    match time {
        Some(time) => processor.settings.timed_action_overrides.insert(binding.into(), time),
        None => processor.settings.timed_action_overrides.remove(binding),
    };
    Ok(())
}

pub async fn change_plug_schedule(
    processor: &mut EventProcessor,
    repository: &impl SettingsRepository,
    device: &str,
    plan: Option<PlugSchedulePlan>,
) -> Result<(), String> {
    let value = plan.map(parse_plug_plan).transpose()?;
    let bindings = match &value {
        Some(value) => Some(processor.validate_plug_override(device, value)?),
        None => {
            processor.validate_plug(device)?;
            None
        }
    };
    repository
        .set_plug_schedule(device, value.as_ref())
        .await
        .map_err(|error| format!("Could not save schedule for {device}: {error}"))?;
    processor.apply_plug_override(device, value.zip(bindings));
    Ok(())
}
