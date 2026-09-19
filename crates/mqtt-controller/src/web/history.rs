use std::future::Future;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use mqtt_controller_wire::{FullStateSnapshot, PlugPowerHistoryPoint, ValveHistoryPoint};
use tokio::sync::{mpsc, oneshot};
use turso::{Builder, Database, params};

use super::server::WsCommand;

pub const HISTORY_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
pub const SAMPLE_INTERVAL_SECS: u64 = 60;
const MAX_ENERGY_SAMPLE_GAP_MS: i64 = 90_000;

#[derive(Debug, Clone)]
pub struct ValveSample {
    pub device: String,
    pub point: ValveHistoryPoint,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlugPowerSample {
    pub device: String,
    pub point: PlugPowerHistoryPoint,
}

pub trait HistoryRepository: Send + Sync {
    fn record(
        &self,
        samples: &[ValveSample],
        now_ms: i64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn fetch(
        &self,
        device: &str,
        now_ms: i64,
    ) -> impl Future<Output = anyhow::Result<Vec<ValveHistoryPoint>>> + Send;
    fn record_power(
        &self,
        samples: &[PlugPowerSample],
        now_ms: i64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn fetch_power(
        &self,
        device: &str,
        now_ms: i64,
    ) -> impl Future<Output = anyhow::Result<Vec<PlugPowerHistoryPoint>>> + Send;
}

#[derive(Clone)]
pub struct SqliteHistory {
    db: Database,
}

impl SqliteHistory {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        let path = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("history path must be UTF-8"))?;
        let db = Builder::new_local(path).build().await?;
        let connection = db.connect()?;
        crate::audit::apply_pragmas(&connection).await?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS valve_history (device TEXT NOT NULL, ts_ms INTEGER NOT NULL, data TEXT NOT NULL, PRIMARY KEY (device, ts_ms))",
            (),
        ).await?;
        connection
            .execute(
                "CREATE INDEX IF NOT EXISTS valve_history_time ON valve_history(ts_ms)",
                (),
            )
            .await?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS plug_power_history (device TEXT NOT NULL, ts_ms INTEGER NOT NULL, data TEXT NOT NULL, PRIMARY KEY (device, ts_ms))",
            (),
        ).await?;
        connection
            .execute(
                "CREATE INDEX IF NOT EXISTS plug_power_history_time ON plug_power_history(ts_ms)",
                (),
            )
            .await?;
        Ok(Self { db })
    }
}

impl HistoryRepository for SqliteHistory {
    async fn record(&self, samples: &[ValveSample], now_ms: i64) -> anyhow::Result<()> {
        let mut connection = self.db.connect()?;
        let transaction = connection.transaction().await?;
        for sample in samples {
            transaction.execute(
                "INSERT INTO valve_history(device, ts_ms, data) VALUES (?, ?, ?) ON CONFLICT(device, ts_ms) DO UPDATE SET data = excluded.data",
                params![sample.device.as_str(), sample.point.timestamp_epoch_ms, serde_json::to_string(&sample.point)?],
            ).await?;
        }
        transaction
            .execute(
                "DELETE FROM valve_history WHERE ts_ms < ?",
                [now_ms - HISTORY_WINDOW_MS],
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn fetch(&self, device: &str, now_ms: i64) -> anyhow::Result<Vec<ValveHistoryPoint>> {
        let connection = self.db.connect()?;
        let mut rows = connection.query(
            "SELECT data FROM valve_history WHERE device = ? AND ts_ms >= ? AND ts_ms <= ? ORDER BY ts_ms",
            params![device, now_ms - HISTORY_WINDOW_MS, now_ms],
        ).await?;
        let mut points = Vec::new();
        while let Some(row) = rows.next().await? {
            points.push(serde_json::from_str(&row.get::<String>(0)?)?);
        }
        Ok(points)
    }

    async fn record_power(&self, samples: &[PlugPowerSample], now_ms: i64) -> anyhow::Result<()> {
        let mut connection = self.db.connect()?;
        let transaction = connection.transaction().await?;
        for sample in samples {
            transaction.execute(
                "INSERT INTO plug_power_history(device, ts_ms, data) VALUES (?, ?, ?) ON CONFLICT(device, ts_ms) DO UPDATE SET data = excluded.data",
                params![sample.device.as_str(), sample.point.timestamp_epoch_ms, serde_json::to_string(&sample.point)?],
            ).await?;
        }
        transaction
            .execute(
                "DELETE FROM plug_power_history WHERE ts_ms < ?",
                [now_ms - HISTORY_WINDOW_MS],
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn fetch_power(&self, device: &str, now_ms: i64) -> anyhow::Result<Vec<PlugPowerHistoryPoint>> {
        let connection = self.db.connect()?;
        let mut rows = connection.query(
            "SELECT data FROM plug_power_history WHERE device = ? AND ts_ms >= ? AND ts_ms <= ? ORDER BY ts_ms",
            params![device, now_ms - HISTORY_WINDOW_MS, now_ms],
        ).await?;
        let mut points = Vec::new();
        while let Some(row) = rows.next().await? {
            points.push(serde_json::from_str(&row.get::<String>(0)?)?);
        }
        Ok(points)
    }
}

fn sample_bucket(timestamp: i64) -> i64 {
    timestamp.div_euclid(SAMPLE_INTERVAL_SECS as i64 * 1000)
        * SAMPLE_INTERVAL_SECS as i64
        * 1000
}

pub fn samples(snapshot: FullStateSnapshot) -> Vec<ValveSample> {
    let timestamp = snapshot.timestamp_epoch_ms as i64;
    // One row per valve per minute, including across rapid daemon restarts.
    let bucket = sample_bucket(timestamp);
    snapshot
        .heating_zones
        .into_iter()
        .flat_map(|zone| zone.trvs)
        .map(|trv| {
            let observed_at = trv
                .actual
                .as_ref()
                .and_then(|actual| actual.since_ago_ms)
                .map(|age| timestamp.saturating_sub(age as i64));
            ValveSample {
                device: trv.device,
                point: ValveHistoryPoint {
                    timestamp_epoch_ms: bucket,
                    observed_at_epoch_ms: observed_at,
                    local_temperature: trv.local_temperature,
                    reported_setpoint: trv.setpoint,
                    target: trv.target_value,
                    heating_demand: trv.pi_heating_demand,
                    battery: trv.battery,
                    freshness: trv
                        .actual
                        .map_or_else(|| "unknown".into(), |actual| actual.freshness),
                },
            }
        })
        .collect()
}

pub fn plug_power_samples(snapshot: &FullStateSnapshot) -> Vec<PlugPowerSample> {
    let timestamp = snapshot.timestamp_epoch_ms as i64;
    let bucket = sample_bucket(timestamp);
    snapshot
        .plugs
        .iter()
        .map(|plug| PlugPowerSample {
            device: plug.device.clone(),
            point: PlugPowerHistoryPoint {
                timestamp_epoch_ms: bucket,
                power_watts: plug
                    .actual_value
                    .as_ref()
                    .and_then(|actual| actual.power)
                    .or(plug.power_watts),
                freshness: plug
                    .actual
                    .as_ref()
                    .map_or_else(|| "unknown".into(), |actual| actual.freshness.clone()),
            },
        })
        .collect()
}

pub fn estimated_energy_kwh(points: &[PlugPowerHistoryPoint]) -> f64 {
    points
        .windows(2)
        .filter_map(|pair| {
            let previous = &pair[0];
            let current = &pair[1];
            let elapsed_ms = current.timestamp_epoch_ms - previous.timestamp_epoch_ms;
            if elapsed_ms <= 0
                || elapsed_ms > MAX_ENERGY_SAMPLE_GAP_MS
                || previous.freshness != "fresh"
                || current.freshness != "fresh"
            {
                return None;
            }
            Some((previous.power_watts? + current.power_watts?) / 2.0 * elapsed_ms as f64 / 3_600_000_000.0)
        })
        .sum()
}

#[derive(Clone)]
pub struct HeatingHistory {
    repository: SqliteHistory,
    error: Arc<RwLock<Option<String>>>,
}

impl HeatingHistory {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            repository: SqliteHistory::open(path).await?,
            error: Arc::new(RwLock::new(Some(
                "Waiting for the first history sample".into(),
            ))),
        })
    }

    pub async fn fetch(
        &self,
        device: &str,
        now_ms: i64,
    ) -> anyhow::Result<(Vec<ValveHistoryPoint>, Option<String>)> {
        let points = self.repository.fetch(device, now_ms).await?;
        let error = self
            .error
            .read()
            .expect("history status lock poisoned")
            .clone();
        Ok((points, error))
    }

    pub async fn fetch_power(
        &self,
        device: &str,
        now_ms: i64,
    ) -> anyhow::Result<(Vec<PlugPowerHistoryPoint>, Option<String>)> {
        let points = self.repository.fetch_power(device, now_ms).await?;
        let error = self
            .error
            .read()
            .expect("history status lock poisoned")
            .clone();
        Ok((points, error))
    }

    pub fn spawn_sampler(&self, commands: mpsc::Sender<WsCommand>) -> tokio::task::JoinHandle<()> {
        let history = self.clone();
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_secs(SAMPLE_INTERVAL_SECS));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;
                let result = async {
                    let snapshot = tokio::time::timeout(Duration::from_secs(5), async {
                        let (reply, response) = oneshot::channel();
                        commands.send(WsCommand::RequestSnapshot { reply }).await?;
                        Ok::<_, anyhow::Error>(response.await?)
                    })
                    .await??;
                    let now_ms = snapshot.timestamp_epoch_ms as i64;
                    let power_samples = plug_power_samples(&snapshot);
                    history.repository.record(&samples(snapshot), now_ms).await?;
                    history.repository.record_power(&power_samples, now_ms).await
                }
                .await;
                let error = result.err().map(|error| error.to_string());
                if let Some(error) = &error {
                    tracing::error!(%error, "telemetry history sampling failed; retrying at the next sample interval");
                }
                *history.error.write().expect("history status lock poisoned") = error;
                if commands.is_closed() {
                    break;
                }
            }
        })
    }
}

#[cfg(test)]
mod tests;
