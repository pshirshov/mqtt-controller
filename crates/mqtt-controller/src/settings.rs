use std::collections::BTreeSet;
use std::future::Future;
use std::path::Path;
use std::time::Instant;

use anyhow::Context;
use turso::{Builder, Connection, Database, params};

use crate::logic::EventProcessor;

/// User intent, separate from observed device state and motion sessions.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MotionSettings {
    pub disabled_zones: BTreeSet<String>,
}

pub trait SettingsRepository: Send + Sync {
    fn load(&self) -> impl Future<Output = anyhow::Result<MotionSettings>> + Send;
    fn set_motion_enabled(
        &self,
        room: &str,
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
    async fn load(&self) -> anyhow::Result<MotionSettings> {
        let connection = self.db.connect()?;
        let mut rows = connection
            .query("SELECT room FROM motion_settings WHERE enabled = 0", ())
            .await?;
        let mut settings = MotionSettings::default();
        while let Some(row) = rows.next().await? {
            settings.disabled_zones.insert(row.get::<String>(0)?);
        }
        Ok(settings)
    }

    async fn set_motion_enabled(&self, room: &str, enabled: bool) -> anyhow::Result<()> {
        let connection = self.write_connection().await?;
        connection.execute(
            "INSERT INTO motion_settings(room, enabled) VALUES (?, ?) ON CONFLICT(room) DO UPDATE SET enabled = excluded.enabled",
            params![room, i64::from(enabled)],
        ).await?;
        Ok(())
    }
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
