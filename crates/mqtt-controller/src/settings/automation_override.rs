//! Dashboard overrides of deployed light and plug automation: slot
//! schedules, timed binding times and kill-switch parameters.

use std::collections::BTreeMap;

use mqtt_controller_wire::SlotPlan;

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

impl ScheduleOwner<'_> {
    pub(crate) const ROOM_KIND: &'static str = "room";
    pub(crate) const MOTION_RULE_KIND: &'static str = "motion_rule";

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Room(_) => Self::ROOM_KIND,
            Self::MotionRule(_) => Self::MOTION_RULE_KIND,
        }
    }

    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Room(name) | Self::MotionRule(name) => name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KillSwitchOverride {
    pub threshold_watts: f64,
    pub holdoff_secs: u64,
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

pub async fn change_scene_schedule(
    processor: &mut EventProcessor,
    repository: &impl SettingsRepository,
    owner: ScheduleOwner<'_>,
    plans: Option<Vec<SlotPlan>>,
) -> Result<(), String> {
    processor.validate_schedule_owner(owner)?;
    let slots = plans.map(parse_slots).transpose()?;
    if let Some(slots) = &slots {
        processor.validate_schedule_override(owner, slots)?;
    }
    repository
        .set_scene_schedule(owner, slots.as_ref())
        .await
        .map_err(|error| format!("Could not save schedule for {}: {error}", owner.name()))?;
    processor.apply_schedule_override(owner, slots);
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

pub async fn change_kill_switch(
    processor: &mut EventProcessor,
    repository: &impl SettingsRepository,
    binding: &str,
    value: Option<KillSwitchOverride>,
) -> Result<(), String> {
    processor.validate_kill_switch(binding)?;
    if let Some(value) = &value {
        value.validate()?;
    }
    repository
        .set_kill_switch(binding, value.as_ref())
        .await
        .map_err(|error| format!("Could not save kill switch for {binding}: {error}"))?;
    match value {
        Some(value) => processor.settings.kill_switch_overrides.insert(binding.into(), value),
        None => processor.settings.kill_switch_overrides.remove(binding),
    };
    Ok(())
}
