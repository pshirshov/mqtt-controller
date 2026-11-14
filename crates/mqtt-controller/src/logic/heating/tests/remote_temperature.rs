use super::*;
use crate::config::heating::{
    RemoteTemperatureConfig, RemoteTemperaturePolicy, RemoteTemperatureSource,
};
use crate::entities::temperature_sensor::TemperatureSensorState;

const SENSOR_A: &str = "nodon-snsr-a";
const SENSOR_B: &str = "nodon-snsr-b";

fn sensor_dev(ieee: &str) -> DeviceCatalogEntry {
    DeviceCatalogEntry::TemperatureSensor(CommonFields {
        ieee_address: ieee.into(),
        display_name: None,
        room: None,
        description: None,
        options: BTreeMap::new(),
    })
}

fn remote_config(policy: RemoteTemperaturePolicy) -> Config {
    let mut cfg = simple_config();
    cfg.devices.insert(SENSOR_A.into(), sensor_dev("0xa1"));
    cfg.devices.insert(SENSOR_B.into(), sensor_dev("0xb1"));
    cfg.heating.as_mut().unwrap().zones[0].trvs[0].remote_temperature =
        Some(RemoteTemperatureConfig {
            policy,
            sources: vec![
                RemoteTemperatureSource {
                    device: SENSOR_A.into(),
                    max_age_minutes: 25,
                },
                RemoteTemperatureSource {
                    device: SENSOR_B.into(),
                    max_age_minutes: 25,
                },
            ],
        });
    cfg
}

fn observe(
    ep: &mut EventProcessor,
    clock: &FakeClock,
    device: &str,
    temperature: f64,
) -> Vec<Effect> {
    ep.handle_event(Event::TemperatureSensorState {
        device: device.into(),
        temperature,
        ts: clock.now(),
    })
}

fn remote_temperatures(effects: &[Effect], ep: &EventProcessor) -> Vec<f64> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::PublishDeviceSet {
                device,
                payload: Payload::RemoteTemperature { remote_temperature },
            } if ep.topology().device_name(*device) == "trv-bath-1" => Some(*remote_temperature),
            _ => None,
        })
        .collect()
}

#[test]
fn live_observations_apply_all_remote_temperature_policies() {
    for (policy, expected) in [
        (RemoteTemperaturePolicy::Min, 19.0),
        (RemoteTemperaturePolicy::Max, 21.0),
        (RemoteTemperaturePolicy::Average, 20.0),
        (RemoteTemperaturePolicy::Freshest, 19.0),
        (RemoteTemperaturePolicy::Fallback, 21.0),
    ] {
        let cfg = remote_config(policy);
        let (mut ep, clock) = setup(&cfg);
        assert_eq!(
            remote_temperatures(&observe(&mut ep, &clock, SENSOR_A, 21.0), &ep),
            vec![21.0]
        );
        clock.advance(Duration::from_secs(1));
        assert_eq!(
            remote_temperatures(&observe(&mut ep, &clock, SENSOR_B, 19.0), &ep),
            vec![expected],
            "policy {policy:?}",
        );
    }
}

#[test]
fn freshest_uses_configured_source_order_as_timestamp_tie_break() {
    let cfg = remote_config(RemoteTemperaturePolicy::Freshest);
    let (mut ep, clock) = setup(&cfg);
    observe(&mut ep, &clock, SENSOR_B, 19.0);
    observe(&mut ep, &clock, SENSOR_A, 21.0);
    let effects = observe(&mut ep, &clock, SENSOR_B, 18.0);
    assert_eq!(remote_temperatures(&effects, &ep), vec![21.0]);
}

#[test]
fn fallback_respects_25_minute_receipt_freshness_and_source_order() {
    let cfg = remote_config(RemoteTemperaturePolicy::Fallback);
    let (mut ep, clock) = setup(&cfg);
    observe(&mut ep, &clock, SENSOR_A, 21.0);

    clock.advance(Duration::from_secs(25 * 60 - 1));
    let effects = observe(&mut ep, &clock, SENSOR_B, 19.0);
    assert_eq!(remote_temperatures(&effects, &ep), vec![21.0]);

    clock.advance(Duration::from_secs(1));
    let effects = observe(&mut ep, &clock, SENSOR_B, 18.5);
    assert_eq!(remote_temperatures(&effects, &ep), vec![21.0]);

    clock.advance(Duration::from_secs(1));
    let effects = observe(&mut ep, &clock, SENSOR_B, 18.0);
    assert_eq!(remote_temperatures(&effects, &ep), vec![18.0]);
}

#[test]
fn all_stale_sources_have_no_aggregate() {
    let config = RemoteTemperatureConfig {
        policy: RemoteTemperaturePolicy::Average,
        sources: vec![
            RemoteTemperatureSource {
                device: SENSOR_A.into(),
                max_age_minutes: 25,
            },
            RemoteTemperatureSource {
                device: SENSOR_B.into(),
                max_age_minutes: 25,
            },
        ],
    };
    let now = std::time::Instant::now();
    let stale_at = now - Duration::from_secs(25 * 60 + 1);
    let observations = BTreeMap::from([
        (
            SENSOR_A.into(),
            TemperatureSensorState {
                temperature: 21.0,
                received_at: stale_at,
            },
        ),
        (
            SENSOR_B.into(),
            TemperatureSensorState {
                temperature: 19.0,
                received_at: stale_at,
            },
        ),
    ]);
    assert_eq!(
        super::super::remote_temperature::aggregate_remote_temperature(&config, &observations, now),
        None,
    );
}

#[test]
fn tick_never_republishes_remote_temperature() {
    let cfg = remote_config(RemoteTemperaturePolicy::Min);
    let (mut ep, clock) = setup(&cfg);
    assert_eq!(
        remote_temperatures(&observe(&mut ep, &clock, SENSOR_A, 20.0), &ep),
        vec![20.0]
    );
    clock.advance(Duration::from_secs(5));
    assert!(remote_temperatures(&tick(&mut ep), &ep).is_empty());
}

#[test]
fn invalid_aggregate_is_not_published_but_raw_observation_is_kept() {
    let cfg = remote_config(RemoteTemperaturePolicy::Freshest);
    let (mut ep, clock) = setup(&cfg);
    let effects = observe(&mut ep, &clock, SENSOR_A, 36.0);
    assert!(remote_temperatures(&effects, &ep).is_empty());
    assert_eq!(ep.world().temperature_sensors[SENSOR_A].temperature, 36.0);
}

#[test]
fn snapshot_exposes_raw_sources_and_controller_aggregate() {
    let cfg = remote_config(RemoteTemperaturePolicy::Average);
    let (mut ep, clock) = setup(&cfg);
    observe(&mut ep, &clock, SENSOR_A, 20.0);
    clock.advance(Duration::from_secs(1));
    observe(&mut ep, &clock, SENSOR_B, 21.0);

    let snapshot = crate::web::snapshot::build_full_snapshot(&ep, clock.now());
    let remote = snapshot.heating_zones[0].trvs[0]
        .remote_temperature
        .as_ref()
        .unwrap();
    assert_eq!(remote.policy, "average");
    assert_eq!(remote.aggregate_temperature, Some(20.5));
    assert_eq!(remote.sources.len(), 2);
    assert_eq!(remote.sources[0].device, SENSOR_A);
    assert_eq!(remote.sources[0].temperature, Some(20.0));
    assert_eq!(remote.sources[0].actual.as_ref().unwrap().freshness, "fresh");
    assert_eq!(remote.sources[1].device, SENSOR_B);
    assert_eq!(remote.sources[1].temperature, Some(21.0));
}

#[test]
fn topology_validates_remote_temperature_contract() {
    let assert_error = |cfg: Config, expected: &str| {
        let error = Topology::build(&cfg).unwrap_err().to_string();
        assert!(
            error.contains(expected),
            "expected {expected:?}, got {error:?}"
        );
    };

    let mut cfg = remote_config(RemoteTemperaturePolicy::Min);
    cfg.heating.as_mut().unwrap().zones[0].trvs[0]
        .remote_temperature
        .as_mut()
        .unwrap()
        .sources
        .clear();
    assert_error(cfg, "must contain at least one source");

    let mut cfg = remote_config(RemoteTemperaturePolicy::Min);
    cfg.heating.as_mut().unwrap().zones[0].trvs[0]
        .remote_temperature
        .as_mut()
        .unwrap()
        .sources[0]
        .max_age_minutes = 0;
    assert_error(cfg, "max_age_minutes must be > 0");

    let mut cfg = remote_config(RemoteTemperaturePolicy::Min);
    let duplicate = cfg.heating.as_ref().unwrap().zones[0].trvs[0]
        .remote_temperature
        .as_ref()
        .unwrap()
        .sources[0]
        .clone();
    cfg.heating.as_mut().unwrap().zones[0].trvs[0]
        .remote_temperature
        .as_mut()
        .unwrap()
        .sources
        .push(duplicate);
    assert_error(cfg, "duplicate source");

    let mut cfg = remote_config(RemoteTemperaturePolicy::Min);
    cfg.heating.as_mut().unwrap().zones[0].trvs[0]
        .remote_temperature
        .as_mut()
        .unwrap()
        .sources[0]
        .device = "missing".into();
    assert_error(cfg, "is not in the device catalog");

    let mut cfg = remote_config(RemoteTemperaturePolicy::Min);
    cfg.heating.as_mut().unwrap().zones[0].trvs[0]
        .remote_temperature
        .as_mut()
        .unwrap()
        .sources[0]
        .device = "wt-bath".into();
    assert_error(cfg, "is a wall-thermostat, not a temperature-sensor");

    let mut cfg = remote_config(RemoteTemperaturePolicy::Min);
    cfg.devices
        .insert("trv-bath-1".into(), trv_dev_sonoff("0xaa"));
    assert_error(cfg, "remote_temperature is only supported by Bosch BTH-RA");
}
