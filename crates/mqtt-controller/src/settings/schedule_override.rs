use mqtt_controller_wire::{ScheduleRange, ScheduleWeekday, ValveSchedule};
use std::collections::BTreeMap;

use crate::config::heating::TemperatureSchedule;
use crate::logic::EventProcessor;

use super::SettingsRepository;

/// Periods are listed in time order; the configuration may list them in
/// any order, since coverage validation sorts them itself.
pub fn schedule_to_wire(schedule: &TemperatureSchedule) -> ValveSchedule {
    let mut days: BTreeMap<ScheduleWeekday, Vec<ScheduleRange>> =
        serde_json::from_value(serde_json::to_value(schedule).expect("valid schedule must serialize"))
            .expect("config and wire schedule formats must agree");
    for ranges in days.values_mut() {
        ranges.sort_by(|a, b| a.start.cmp(&b.start));
    }
    ValveSchedule { days }
}

pub async fn change_valve_schedule(
    processor: &mut EventProcessor,
    repository: &impl SettingsRepository,
    device: &str,
    schedule: Option<ValveSchedule>,
) -> Result<(), String> {
    processor.validate_heat_demand_device(device)?;
    let parsed = schedule.map(|schedule| {
        let parsed: TemperatureSchedule = serde_json::from_value(
            serde_json::to_value(schedule.days).expect("wire schedule must serialize")
        ).map_err(|error| format!("Invalid valve schedule: {error}"))?;
        parsed.validate(device).map_err(|error| error.to_string())?;
        Ok::<TemperatureSchedule, String>(parsed)
    }).transpose()?;
    repository.set_valve_schedule(device, parsed.as_ref()).await
        .map_err(|error| format!("Could not save schedule for {device}: {error}"))?;
    if let Some(schedule) = parsed {
        processor.settings.schedule_overrides.insert(device.into(), schedule);
    } else {
        processor.settings.schedule_overrides.remove(device);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::schedule_to_wire;
    use crate::config::heating::TemperatureSchedule;

    #[test]
    fn wire_schedule_lists_periods_in_time_order() {
        let day = serde_json::json!([
            {"start": "06:00", "end": "23:00", "temperature": 21},
            {"start": "00:00", "end": "06:00", "temperature": 18},
            {"start": "23:00", "end": "24:00", "temperature": 18}
        ]);
        let days: serde_json::Map<String, serde_json::Value> = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"]
            .into_iter().map(|name| (name.to_string(), day.clone())).collect();
        let schedule: TemperatureSchedule = serde_json::from_value(serde_json::Value::Object(days)).unwrap();
        schedule.validate("test").unwrap();
        let wire = schedule_to_wire(&schedule);
        let starts: Vec<&str> = wire.days.values().next().unwrap().iter().map(|range| range.start.as_str()).collect();
        assert_eq!(starts, vec!["00:00", "06:00", "23:00"]);
    }
}
