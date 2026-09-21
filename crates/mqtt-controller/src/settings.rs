use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::Path;
use std::time::Instant;

use anyhow::Context;
use turso::{Builder, Connection, Database, params};

use crate::logic::EventProcessor;

mod boost;
pub use boost::{BoostChange, ValveBoost, change_valve_boost};

/// User intent, separate from observed device state and automation sessions.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ControlSettings {
    pub disabled_zones: BTreeSet<String>,
    pub disabled_heat_demand: BTreeSet<String>,
    pub boosts: BTreeMap<String, ValveBoost>,
}

pub trait SettingsRepository: Send + Sync {
    fn set_valve_boost(&self, device: &str, boost: Option<&ValveBoost>)
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
        Ok(settings)
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
