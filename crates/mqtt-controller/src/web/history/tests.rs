use std::collections::BTreeMap;
use std::sync::Mutex;

use super::*;

#[derive(Default)]
struct MemoryHistory {
    valves: Mutex<BTreeMap<(String, i64), ValveHistoryPoint>>,
    plugs: Mutex<BTreeMap<(String, i64), PlugPowerHistoryPoint>>,
}

impl HistoryRepository for MemoryHistory {
    async fn record(&self, samples: &[ValveSample], now_ms: i64) -> anyhow::Result<()> {
        let mut rows = self.valves.lock().unwrap();
        for sample in samples {
            rows.insert(
                (sample.device.clone(), sample.point.timestamp_epoch_ms),
                sample.point.clone(),
            );
        }
        rows.retain(|(_, timestamp), _| *timestamp >= now_ms - HISTORY_WINDOW_MS);
        Ok(())
    }
    async fn fetch(&self, device: &str, now_ms: i64) -> anyhow::Result<Vec<ValveHistoryPoint>> {
        Ok(self
            .valves
            .lock()
            .unwrap()
            .iter()
            .filter(|((name, timestamp), _)| {
                name == device && *timestamp >= now_ms - HISTORY_WINDOW_MS && *timestamp <= now_ms
            })
            .map(|(_, point)| point.clone())
            .collect())
    }

    async fn record_power(&self, samples: &[PlugPowerSample], now_ms: i64) -> anyhow::Result<()> {
        let mut rows = self.plugs.lock().unwrap();
        for sample in samples {
            rows.insert(
                (sample.device.clone(), sample.point.timestamp_epoch_ms),
                sample.point.clone(),
            );
        }
        rows.retain(|(_, timestamp), _| *timestamp >= now_ms - HISTORY_WINDOW_MS);
        Ok(())
    }

    async fn fetch_power(&self, device: &str, now_ms: i64) -> anyhow::Result<Vec<PlugPowerHistoryPoint>> {
        Ok(self
            .plugs
            .lock()
            .unwrap()
            .iter()
            .filter(|((name, timestamp), _)| {
                name == device && *timestamp >= now_ms - HISTORY_WINDOW_MS && *timestamp <= now_ms
            })
            .map(|(_, point)| point.clone())
            .collect())
    }
}

fn sample(device: &str, timestamp: i64, temperature: Option<f64>) -> ValveSample {
    ValveSample {
        device: device.into(),
        point: ValveHistoryPoint {
            timestamp_epoch_ms: timestamp,
            observed_at_epoch_ms: temperature.map(|_| timestamp - 1000),
            local_temperature: temperature,
            reported_setpoint: Some(20.0),
            target: Some(mqtt_controller_wire::TrvTargetValue::Setpoint { temperature: 21.0 }),
            heating_demand: Some(40),
            running_state: mqtt_controller_wire::TrvRunningState::Heat,
            battery: Some(80),
            freshness: "fresh".into(),
        },
    }
}

async fn history_contract(repository: &impl HistoryRepository) {
    let now = HISTORY_WINDOW_MS * 2;
    repository
        .record(
            &[
                sample("bathroom", now - HISTORY_WINDOW_MS - 1, Some(18.0)),
                sample("bathroom", now - HISTORY_WINDOW_MS, Some(19.0)),
                sample("bedroom", now, Some(17.0)),
                sample("bathroom", now, None),
                sample("bathroom", now + 1000, Some(30.0)),
            ],
            now,
        )
        .await
        .unwrap();
    let rows = repository.fetch("bathroom", now).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].local_temperature, Some(19.0));
    assert_eq!(rows[1].local_temperature, None);
    assert_eq!(
        rows[1].target,
        Some(mqtt_controller_wire::TrvTargetValue::Setpoint { temperature: 21.0 })
    );
    assert_eq!(repository.fetch("missing", now).await.unwrap(), []);
    repository
        .record(&[sample("bathroom", now, Some(22.0))], now)
        .await
        .unwrap();
    let rows = repository.fetch("bathroom", now).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].local_temperature, Some(22.0));
    repository
        .record(&[], now + HISTORY_WINDOW_MS + 1001)
        .await
        .unwrap();
    assert!(
        repository
            .fetch("bathroom", now + HISTORY_WINDOW_MS + 1001)
            .await
            .unwrap()
            .is_empty()
    );
}

fn power_sample(device: &str, timestamp: i64, power_watts: Option<f64>) -> PlugPowerSample {
    PlugPowerSample {
        device: device.into(),
        point: PlugPowerHistoryPoint {
            timestamp_epoch_ms: timestamp,
            power_watts,
            freshness: "fresh".into(),
        },
    }
}

async fn power_history_contract(repository: &impl HistoryRepository) {
    let now = HISTORY_WINDOW_MS * 2;
    repository
        .record_power(
            &[
                power_sample("printer", now - HISTORY_WINDOW_MS - 1, Some(10.0)),
                power_sample("printer", now - HISTORY_WINDOW_MS, Some(20.0)),
                power_sample("other", now, Some(30.0)),
                power_sample("printer", now, Some(40.0)),
            ],
            now,
        )
        .await
        .unwrap();
    let rows = repository.fetch_power("printer", now).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].power_watts, Some(20.0));
    assert_eq!(rows[1].power_watts, Some(40.0));
    repository
        .record_power(&[power_sample("printer", now, Some(50.0))], now)
        .await
        .unwrap();
    assert_eq!(repository.fetch_power("printer", now).await.unwrap()[1].power_watts, Some(50.0));
}

#[tokio::test]
async fn memory_history_contract() {
    let repository = MemoryHistory::default();
    history_contract(&repository).await;
    power_history_contract(&repository).await;
}

#[tokio::test]
async fn sqlite_history_contract() {
    let directory = tempfile::tempdir().unwrap();
    let repository = SqliteHistory::open(&directory.path().join("history.db"))
        .await
        .unwrap();
    history_contract(&repository).await;
    power_history_contract(&repository).await;
}

#[test]
fn energy_estimate_is_unknown_without_a_measurable_interval() {
    let point = power_sample("printer", 0, Some(1000.0)).point;
    for points in [vec![], vec![point]] {
        assert_eq!(
            estimate_energy(&points),
            None,
            "absence of measured intervals must not be reported as zero consumption",
        );
    }
}

#[test]
fn energy_estimate_integrates_fresh_samples_without_bridging_gaps() {
    let mut points = vec![
        power_sample("printer", 0, Some(1000.0)).point,
        power_sample("printer", 60_000, Some(1000.0)).point,
        power_sample("printer", 120_000, Some(1000.0)).point,
        power_sample("printer", 300_000, Some(1000.0)).point,
    ];
    let estimate = estimate_energy(&points).unwrap();
    assert!((estimate.kwh - (2.0 / 60.0)).abs() < 1e-12);
    assert_eq!(estimate.observed_ms, 120_000);
    points[1].freshness = "stale".into();
    assert_eq!(estimate_energy(&points), None);
}

#[test]
fn energy_estimate_distinguishes_measured_zero_and_missing_readings() {
    let mut points = vec![
        power_sample("printer", 0, Some(0.0)).point,
        power_sample("printer", 60_000, Some(0.0)).point,
    ];
    assert_eq!(estimate_energy(&points), Some(EnergyEstimate { kwh: 0.0, observed_ms: 60_000 }));
    points[1].power_watts = None;
    assert_eq!(estimate_energy(&points), None);
}

#[tokio::test]
async fn history_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.db");
    let point = sample("valve", HISTORY_WINDOW_MS, Some(21.5));
    {
        let repository = SqliteHistory::open(&path).await.unwrap();
        repository
            .record(&[point.clone()], HISTORY_WINDOW_MS)
            .await
            .unwrap();
    }
    let repository = SqliteHistory::open(&path).await.unwrap();
    assert_eq!(
        repository.fetch("valve", HISTORY_WINDOW_MS).await.unwrap(),
        [point.point]
    );
}

fn snapshot(timestamp: u64) -> FullStateSnapshot {
    serde_json::from_value(serde_json::json!({
        "timestamp_epoch_ms": timestamp, "rooms": [], "plugs": [{
            "device": "printer", "on": true, "idle_since_ago_ms": null,
            "room": "office", "display_name": "3d printer",
            "power_watts": 120.0,
            "power_actual": { "freshness": "fresh", "since_ago_ms": 1000 },
            "actual": { "freshness": "stale", "since_ago_ms": 601_000 },
            "actual_value": { "on": true, "power": 120.0 }
        }],
        "heating_zones": [{
            "name": "upstairs", "relay_device": "relay", "relay_on": false,
            "relay_state_known": false, "relay_temperature": null,
            "trvs": [{
                "device": "ensuite", "local_temperature": null, "setpoint": 18.0,
                "pi_heating_demand": 0, "battery": 0, "running_state": "idle", "inhibited": false,
                "target_value": { "kind": "setpoint", "temperature": 21.0 },
                "actual": { "freshness": "stale", "since_ago_ms": 3600000 }
            }]
        }]
    }))
    .unwrap()
}

#[test]
fn sampling_keeps_target_actual_zero_and_observation_time_distinct() {
    let now = HISTORY_WINDOW_MS as u64 + 23_000;
    let points = samples(snapshot(now));
    assert_eq!(points.len(), 1);
    let point = &points[0].point;
    assert_eq!(point.timestamp_epoch_ms, HISTORY_WINDOW_MS);
    assert_eq!(point.observed_at_epoch_ms, Some(now as i64 - 3_600_000));
    assert_eq!(point.local_temperature, None);
    assert_eq!(point.reported_setpoint, Some(18.0));
    assert_eq!(
        point.target,
        Some(mqtt_controller_wire::TrvTargetValue::Setpoint { temperature: 21.0 })
    );
    assert_eq!(point.battery, Some(0));
    assert_eq!(point.heating_demand, Some(0));
    assert_eq!(point.freshness, "stale");
}

// Specified: binary TRV activity is persisted independently of numeric PI demand.
#[test]
fn sampling_preserves_running_state_without_synthesizing_numeric_demand() {
    let now = HISTORY_WINDOW_MS as u64 + 23_000;
    let mut state = snapshot(now);
    let trv = &mut state.heating_zones[0].trvs[0];
    trv.pi_heating_demand = None;
    trv.running_state = mqtt_controller_wire::TrvRunningState::Heat;
    let point = serde_json::to_value(&samples(state)[0].point).unwrap();
    assert_eq!(point["running_state"], "heat");
    assert_eq!(point["heating_demand"], serde_json::Value::Null);
}

#[test]
fn legacy_history_without_running_state_decodes_as_unknown() {
    let legacy = serde_json::json!({
        "timestamp_epoch_ms": 1000,
        "observed_at_epoch_ms": 900,
        "local_temperature": 20.0,
        "reported_setpoint": 21.0,
        "target": null,
        "heating_demand": null,
        "battery": 80,
        "freshness": "fresh"
    });
    let point: ValveHistoryPoint = serde_json::from_value(legacy).unwrap();
    assert_eq!(serde_json::to_value(point).unwrap()["running_state"], "unknown");
}

#[test]
fn sampling_records_plug_power_with_meter_freshness() {
    let now = HISTORY_WINDOW_MS as u64 + 23_000;
    assert_eq!(
        plug_power_samples(&snapshot(now)),
        [PlugPowerSample {
            device: "printer".into(),
            point: PlugPowerHistoryPoint {
                timestamp_epoch_ms: HISTORY_WINDOW_MS,
                power_watts: Some(120.0),
                freshness: "fresh".into(),
            },
        }]
    );
}

#[tokio::test]
async fn sampler_records_a_controller_snapshot_and_clears_startup_status() {
    let directory = tempfile::tempdir().unwrap();
    let history = HeatingHistory::open(&directory.path().join("history.db"))
        .await
        .unwrap();
    let (commands, mut requests) = mpsc::channel(1);
    let sampler = history.spawn_sampler(commands);
    let request = tokio::time::timeout(Duration::from_secs(1), requests.recv())
        .await
        .unwrap()
        .unwrap();
    let WsCommand::RequestSnapshot { reply } = request else {
        panic!("expected a snapshot request")
    };
    reply.send(snapshot(HISTORY_WINDOW_MS as u64)).unwrap();
    let points = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let (points, error) = history.fetch("ensuite", HISTORY_WINDOW_MS).await.unwrap();
            if error.is_none() {
                break points;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        points,
        samples(snapshot(HISTORY_WINDOW_MS as u64))
            .into_iter()
            .map(|sample| sample.point)
            .collect::<Vec<_>>()
    );
    let (power, error) = history.fetch_power("printer", HISTORY_WINDOW_MS).await.unwrap();
    assert!(error.is_none());
    assert_eq!(power, plug_power_samples(&snapshot(HISTORY_WINDOW_MS as u64)).into_iter().map(|sample| sample.point).collect::<Vec<_>>());
    sampler.abort();
}

#[tokio::test]
async fn sampler_reports_timeout_when_the_controller_queue_is_full() {
    let directory = tempfile::tempdir().unwrap();
    let history = HeatingHistory::open(&directory.path().join("history.db"))
        .await
        .unwrap();
    let (commands, _requests) = mpsc::channel(1);
    let (reply, _response) = oneshot::channel();
    commands
        .send(WsCommand::RequestSnapshot { reply })
        .await
        .unwrap();
    let sampler = history.spawn_sampler(commands);
    tokio::time::sleep(Duration::from_millis(5200)).await;
    let (_, error) = history.fetch("ensuite", HISTORY_WINDOW_MS).await.unwrap();
    sampler.abort();
    assert!(
        error.unwrap().contains("deadline"),
        "a full queue must surface a sampling timeout"
    );
}
