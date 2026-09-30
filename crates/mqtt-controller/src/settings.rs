use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::Path;
use std::time::Instant;

use anyhow::Context;
use turso::{Builder, Connection, Database, params};

use crate::logic::EventProcessor;

mod boost;
pub use boost::{BoostChange, ValveBoost, change_valve_boost};
mod schedule_override;
pub use schedule_override::{change_valve_schedule, schedule_to_wire};
mod automation_override;
pub use automation_override::{
    KillSwitchOverride, RoomScheduleOverride, RoomSchedulePlan, ScheduleOwner, change_kill_switch,
    change_motion_schedule, change_room_schedule, change_timed_action_time, slot_plans,
};
use crate::config::heating::TemperatureSchedule;
use crate::config::scenes::{Slot, SlotName};
use crate::config::time_expr::TimeExpr;

/// User intent, separate from observed device state and automation sessions.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ControlSettings {
    pub disabled_zones: BTreeSet<String>,
    pub disabled_heat_demand: BTreeSet<String>,
    pub boosts: BTreeMap<String, ValveBoost>,
    pub schedule_overrides: BTreeMap<String, TemperatureSchedule>,
    pub room_schedule_overrides: BTreeMap<String, RoomScheduleOverride>,
    pub motion_schedule_overrides: BTreeMap<String, BTreeMap<SlotName, Slot>>,
    pub timed_action_overrides: BTreeMap<String, TimeExpr>,
    pub kill_switch_overrides: BTreeMap<String, KillSwitchOverride>,
}

pub trait SettingsRepository: Send + Sync {
    fn set_valve_boost(&self, device: &str, boost: Option<&ValveBoost>)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn set_valve_schedule(&self, device: &str, schedule: Option<&TemperatureSchedule>)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn set_room_schedule(&self, room: &str, value: Option<&RoomScheduleOverride>)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn set_motion_schedule(&self, rule: &str, slots: Option<&BTreeMap<SlotName, Slot>>)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn set_timed_action_time(&self, binding: &str, time: Option<&TimeExpr>)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn set_kill_switch(&self, binding: &str, value: Option<&KillSwitchOverride>)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn load(&self) -> impl Future<Output = anyhow::Result<ControlSettings>> + Send;
    fn set_motion_enabled(
        &self,
        room: &str,
        enabled: bool,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn set_heat_demand_enabled(
        &self,
        device: &str,
        enabled: bool,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

pub struct SqliteSettings {
    db: Database,
}

impl SqliteSettings {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        let db = Builder::new_local(path.to_str().context("settings path must be UTF-8")?)
            .build()
            .await?;
        let connection = db.connect()?;
        crate::audit::apply_pragmas(&connection).await?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS motion_settings (room TEXT PRIMARY KEY NOT NULL, enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)))",
            (),
        ).await?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS heat_demand_settings (device TEXT PRIMARY KEY NOT NULL, enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)))",
            (),
        ).await?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS valve_boosts (device TEXT PRIMARY KEY NOT NULL, temperature REAL NOT NULL CHECK(temperature BETWEEN 5 AND 30), ends_at_ms INTEGER NOT NULL CHECK(ends_at_ms > 0))",
            (),
        ).await?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS valve_schedule_overrides (device TEXT PRIMARY KEY NOT NULL, schedule_json TEXT NOT NULL)",
            (),
        ).await?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS room_schedule_overrides (room TEXT PRIMARY KEY NOT NULL, override_json TEXT NOT NULL)",
            (),
        ).await?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS motion_schedule_overrides (rule TEXT PRIMARY KEY NOT NULL, slots_json TEXT NOT NULL)",
            (),
        ).await?;
        // Earlier releases kept room and motion slot overrides in one table.
        // Room rows lack switch steps and cannot be applied any more.
        let mut rows = connection.query(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'scene_schedule_overrides'", (),
        ).await?;
        let legacy_table = rows.next().await?.is_some();
        drop(rows);
        if legacy_table {
            let mut rows = connection.query("SELECT owner FROM scene_schedule_overrides WHERE owner_kind = 'room'", ()).await?;
            while let Some(row) = rows.next().await? {
                tracing::warn!(room = row.get::<String>(0)?,
                    "dropping light schedule override saved by an earlier release; save it again from the dashboard");
            }
            drop(rows);
            connection.execute(
                "INSERT OR REPLACE INTO motion_schedule_overrides(rule, slots_json) SELECT owner, slots_json FROM scene_schedule_overrides WHERE owner_kind = 'motion_rule'",
                (),
            ).await?;
            connection.execute("DROP TABLE scene_schedule_overrides", ()).await?;
        }
        connection.execute(
            "CREATE TABLE IF NOT EXISTS timed_action_overrides (binding TEXT PRIMARY KEY NOT NULL, time TEXT NOT NULL)",
            (),
        ).await?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS kill_switch_overrides (binding TEXT PRIMARY KEY NOT NULL, threshold_watts REAL NOT NULL CHECK(threshold_watts > 0), holdoff_secs INTEGER NOT NULL CHECK(holdoff_secs > 0))",
            (),
        ).await?;
        Ok(Self { db })
    }

    async fn write_connection(&self) -> anyhow::Result<Connection> {
        let connection = self.db.connect()?;
        let mut rows = connection.query("PRAGMA synchronous = FULL", ()).await?;
        while rows.next().await?.is_some() {}
        drop(rows);
        Ok(connection)
    }
}

impl SettingsRepository for SqliteSettings {
    async fn load(&self) -> anyhow::Result<ControlSettings> {
        let connection = self.db.connect()?;
        let mut rows = connection
            .query("SELECT room FROM motion_settings WHERE enabled = 0", ())
            .await?;
        let mut settings = ControlSettings::default();
        while let Some(row) = rows.next().await? {
            settings.disabled_zones.insert(row.get::<String>(0)?);
        }
        let mut rows = connection.query("SELECT device FROM heat_demand_settings WHERE enabled = 0", ()).await?;
        while let Some(row) = rows.next().await? {
            settings.disabled_heat_demand.insert(row.get::<String>(0)?);
        }
        let mut rows = connection.query("SELECT device, temperature, ends_at_ms FROM valve_boosts", ()).await?;
        while let Some(row) = rows.next().await? {
            let temperature = row.get::<f64>(1)?;
            boost::validate_temperature(temperature).map_err(anyhow::Error::msg)?;
            settings.boosts.insert(row.get::<String>(0)?, ValveBoost {
                temperature,
                ends_at_epoch_ms: u64::try_from(row.get::<i64>(2)?)?,
            });
        }
        let mut rows = connection.query("SELECT device, schedule_json FROM valve_schedule_overrides", ()).await?;
        while let Some(row) = rows.next().await? {
            let device = row.get::<String>(0)?;
            let schedule: TemperatureSchedule = serde_json::from_str(&row.get::<String>(1)?)?;
            schedule.validate(&device)?;
            settings.schedule_overrides.insert(device, schedule);
        }
        let mut rows = connection.query("SELECT room, override_json FROM room_schedule_overrides", ()).await?;
        while let Some(row) = rows.next().await? {
            let value: RoomScheduleOverride = serde_json::from_str(&row.get::<String>(1)?)?;
            settings.room_schedule_overrides.insert(row.get::<String>(0)?, value);
        }
        let mut rows = connection.query("SELECT rule, slots_json FROM motion_schedule_overrides", ()).await?;
        while let Some(row) = rows.next().await? {
            let slots: BTreeMap<SlotName, Slot> = serde_json::from_str(&row.get::<String>(1)?)?;
            settings.motion_schedule_overrides.insert(row.get::<String>(0)?, slots);
        }
        let mut rows = connection.query("SELECT binding, time FROM timed_action_overrides", ()).await?;
        while let Some(row) = rows.next().await? {
            settings.timed_action_overrides.insert(row.get::<String>(0)?, row.get::<String>(1)?.parse()?);
        }
        let mut rows = connection.query("SELECT binding, threshold_watts, holdoff_secs FROM kill_switch_overrides", ()).await?;
        while let Some(row) = rows.next().await? {
            let value = KillSwitchOverride {
                threshold_watts: row.get::<f64>(1)?,
                holdoff_secs: u64::try_from(row.get::<i64>(2)?)?,
            };
            value.validate().map_err(anyhow::Error::msg)?;
            settings.kill_switch_overrides.insert(row.get::<String>(0)?, value);
        }
        Ok(settings)
    }

    async fn set_room_schedule(&self, room: &str, value: Option<&RoomScheduleOverride>) -> anyhow::Result<()> {
        let connection = self.write_connection().await?;
        if let Some(value) = value {
            connection.execute(
                "INSERT INTO room_schedule_overrides(room, override_json) VALUES (?, ?) ON CONFLICT(room) DO UPDATE SET override_json = excluded.override_json",
                params![room, serde_json::to_string(value)?],
            ).await?;
        } else {
            connection.execute("DELETE FROM room_schedule_overrides WHERE room = ?", [room]).await?;
        }
        Ok(())
    }

    async fn set_motion_schedule(&self, rule: &str, slots: Option<&BTreeMap<SlotName, Slot>>) -> anyhow::Result<()> {
        let connection = self.write_connection().await?;
        if let Some(slots) = slots {
            connection.execute(
                "INSERT INTO motion_schedule_overrides(rule, slots_json) VALUES (?, ?) ON CONFLICT(rule) DO UPDATE SET slots_json = excluded.slots_json",
                params![rule, serde_json::to_string(slots)?],
            ).await?;
        } else {
            connection.execute("DELETE FROM motion_schedule_overrides WHERE rule = ?", [rule]).await?;
        }
        Ok(())
    }

    async fn set_timed_action_time(&self, binding: &str, time: Option<&TimeExpr>) -> anyhow::Result<()> {
        let connection = self.write_connection().await?;
        if let Some(time) = time {
            connection.execute(
                "INSERT INTO timed_action_overrides(binding, time) VALUES (?, ?) ON CONFLICT(binding) DO UPDATE SET time = excluded.time",
                params![binding, time.to_string()],
            ).await?;
        } else {
            connection.execute("DELETE FROM timed_action_overrides WHERE binding = ?", [binding]).await?;
        }
        Ok(())
    }

    async fn set_kill_switch(&self, binding: &str, value: Option<&KillSwitchOverride>) -> anyhow::Result<()> {
        let connection = self.write_connection().await?;
        if let Some(value) = value {
            value.validate().map_err(anyhow::Error::msg)?;
            connection.execute(
                "INSERT INTO kill_switch_overrides(binding, threshold_watts, holdoff_secs) VALUES (?, ?, ?) ON CONFLICT(binding) DO UPDATE SET threshold_watts = excluded.threshold_watts, holdoff_secs = excluded.holdoff_secs",
                params![binding, value.threshold_watts, i64::try_from(value.holdoff_secs)?],
            ).await?;
        } else {
            connection.execute("DELETE FROM kill_switch_overrides WHERE binding = ?", [binding]).await?;
        }
        Ok(())
    }

    async fn set_valve_schedule(&self, device: &str, schedule: Option<&TemperatureSchedule>) -> anyhow::Result<()> {
        let connection = self.write_connection().await?;
        if let Some(schedule) = schedule {
            schedule.validate(device)?;
            connection.execute(
                "INSERT INTO valve_schedule_overrides(device, schedule_json) VALUES (?, ?) ON CONFLICT(device) DO UPDATE SET schedule_json = excluded.schedule_json",
                params![device, serde_json::to_string(schedule)?],
            ).await?;
        } else {
            connection.execute("DELETE FROM valve_schedule_overrides WHERE device = ?", [device]).await?;
        }
        Ok(())
    }

    async fn set_valve_boost(&self, device: &str, boost: Option<&ValveBoost>) -> anyhow::Result<()> {
        let connection = self.write_connection().await?;
        if let Some(boost) = boost {
            boost::validate_temperature(boost.temperature).map_err(anyhow::Error::msg)?;
            connection.execute(
                "INSERT INTO valve_boosts(device, temperature, ends_at_ms) VALUES (?, ?, ?) ON CONFLICT(device) DO UPDATE SET temperature = excluded.temperature, ends_at_ms = excluded.ends_at_ms",
                params![device, boost.temperature, i64::try_from(boost.ends_at_epoch_ms)?],
            ).await?;
        } else {
            connection.execute("DELETE FROM valve_boosts WHERE device = ?", [device]).await?;
        }
        Ok(())
    }

    async fn set_motion_enabled(&self, room: &str, enabled: bool) -> anyhow::Result<()> {
        let connection = self.write_connection().await?;
        connection.execute(
            "INSERT INTO motion_settings(room, enabled) VALUES (?, ?) ON CONFLICT(room) DO UPDATE SET enabled = excluded.enabled",
            params![room, i64::from(enabled)],
        ).await?;
        Ok(())
    }

    async fn set_heat_demand_enabled(&self, device: &str, enabled: bool) -> anyhow::Result<()> {
        let connection = self.write_connection().await?;
        connection.execute(
            "INSERT INTO heat_demand_settings(device, enabled) VALUES (?, ?) ON CONFLICT(device) DO UPDATE SET enabled = excluded.enabled",
            params![device, i64::from(enabled)],
        ).await?;
        Ok(())
    }
}

pub async fn set_heat_demand_enabled(
    processor: &mut EventProcessor,
    repository: &impl SettingsRepository,
    device: &str,
    enabled: bool,
) -> Result<(), String> {
    processor.validate_heat_demand_device(device)?;
    repository.set_heat_demand_enabled(device, enabled).await
        .map_err(|error| format!("Could not save heat demand setting for {device}: {error}"))?;
    processor.set_heat_demand_enabled(device, enabled)
}

pub async fn set_motion_enabled(
    processor: &mut EventProcessor,
    repository: &impl SettingsRepository,
    room: &str,
    enabled: bool,
    now: Instant,
) -> Result<(), String> {
    processor.validate_motion_zone(room)?;
    repository
        .set_motion_enabled(room, enabled)
        .await
        .map_err(|error| format!("Could not save motion setting for {room}: {error}"))?;
    processor.set_motion_enabled(room, enabled, now)
}
