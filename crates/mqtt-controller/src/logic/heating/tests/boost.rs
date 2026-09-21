use super::*;
use crate::settings::{
    BoostChange, ControlSettings, SettingsRepository, SqliteSettings, ValveBoost,
    change_valve_boost,
};
use crate::tass::Owner;
use crate::web::snapshot::build_full_snapshot;
use std::sync::Mutex;

const VALVE: &str = "trv-bath-1";

#[derive(Default)]
struct MemorySettings(Mutex<ControlSettings>);

impl SettingsRepository for MemorySettings {
    async fn load(&self) -> anyhow::Result<ControlSettings> {
        Ok(self.0.lock().unwrap().clone())
    }
    async fn set_valve_boost(
        &self,
        device: &str,
        boost: Option<&ValveBoost>,
    ) -> anyhow::Result<()> {
        let mut settings = self.0.lock().unwrap();
        if let Some(boost) = boost {
            settings.boosts.insert(device.into(), *boost);
        } else {
            settings.boosts.remove(device);
        }
        Ok(())
    }
    async fn set_motion_enabled(&self, room: &str, enabled: bool) -> anyhow::Result<()> {
        let mut settings = self.0.lock().unwrap();
        if enabled {
            settings.disabled_zones.remove(room);
        } else {
            settings.disabled_zones.insert(room.into());
        }
        Ok(())
    }
    async fn set_heat_demand_enabled(&self, device: &str, enabled: bool) -> anyhow::Result<()> {
        let mut settings = self.0.lock().unwrap();
        if enabled {
            settings.disabled_heat_demand.remove(device);
        } else {
            settings.disabled_heat_demand.insert(device.into());
        }
        Ok(())
    }
}

struct RejectBoostWrites<'a, R>(&'a R);
impl<R: SettingsRepository> SettingsRepository for RejectBoostWrites<'_, R> {
    async fn load(&self) -> anyhow::Result<ControlSettings> {
        self.0.load().await
    }
    async fn set_valve_boost(&self, _: &str, _: Option<&ValveBoost>) -> anyhow::Result<()> {
        anyhow::bail!("write rejected")
    }
    async fn set_motion_enabled(&self, room: &str, enabled: bool) -> anyhow::Result<()> {
        self.0.set_motion_enabled(room, enabled).await
    }
    async fn set_heat_demand_enabled(&self, device: &str, enabled: bool) -> anyhow::Result<()> {
        self.0.set_heat_demand_enabled(device, enabled).await
    }
}

async fn start(ep: &mut EventProcessor, repository: &impl SettingsRepository, minutes: u16) {
    change_valve_boost(
        ep,
        repository,
        VALVE,
        BoostChange::Start {
            duration_minutes: minutes,
            temperature: 22.0,
        },
    )
    .await
    .unwrap();
}

fn valve(ep: &EventProcessor) -> mqtt_controller_wire::TrvSnapshot {
    build_full_snapshot(ep, ep.clock.now())
        .heating_zones
        .remove(0)
        .trvs
        .remove(0)
}

// Specified × Blackbox × Atomic/Communication: the same contracts and business rules
// run with a manual repository and the production SQLite adapter.
async fn boost_contract(repository: &impl SettingsRepository) {
    assert_eq!(repository.load().await.unwrap(), ControlSettings::default());
    let first = ValveBoost {
        temperature: 22.0,
        ends_at_epoch_ms: 1_700_002_000_000,
    };
    let edited = ValveBoost {
        temperature: 23.5,
        ..first
    };
    repository
        .set_valve_boost("one", Some(&first))
        .await
        .unwrap();
    repository
        .set_valve_boost("two", Some(&first))
        .await
        .unwrap();
    repository
        .set_valve_boost("one", Some(&edited))
        .await
        .unwrap();
    assert_eq!(
        repository.load().await.unwrap().boosts,
        BTreeMap::from([("one".into(), edited), ("two".into(), first)])
    );
    for _ in 0..2 {
        repository.set_valve_boost("one", None).await.unwrap();
    }
    assert_eq!(
        repository.load().await.unwrap().boosts,
        BTreeMap::from([("two".into(), first)])
    );
    repository.set_valve_boost("two", None).await.unwrap();

    let cfg = simple_config();
    let (mut ep, clk) = setup(&cfg);
    tick(&mut ep);
    send_trv_demand(&mut ep, VALVE, 19.0, 0, "idle", 20.0, &clk);
    for minutes in [30, 60, 90, 120, 180, 240, 360] {
        start(&mut ep, repository, minutes).await;
        let boost = valve(&ep).boost.unwrap();
        assert_eq!(boost.temperature, 22.0);
        assert_eq!(boost.remaining_ms, u64::from(minutes) * 60_000);
        assert_eq!(
            boost.ends_at_epoch_ms,
            clk.epoch_millis() + boost.remaining_ms
        );
        change_valve_boost(&mut ep, repository, VALVE, BoostChange::Cancel)
            .await
            .unwrap();
    }
    for (device, minutes, temperature) in [
        ("missing", 30, 22.0),
        ("wt-bath", 30, 22.0),
        (VALVE, 31, 22.0),
        (VALVE, 0, 22.0),
        (VALVE, 30, 4.5),
        (VALVE, 30, 30.5),
        (VALVE, 30, 22.25),
        (VALVE, 30, f64::NAN),
    ] {
        assert!(
            change_valve_boost(
                &mut ep,
                repository,
                device,
                BoostChange::Start {
                    duration_minutes: minutes,
                    temperature
                }
            )
            .await
            .is_err()
        );
        assert!(repository.load().await.unwrap().boosts.is_empty());
    }
    let failing = RejectBoostWrites(repository);
    assert!(
        change_valve_boost(
            &mut ep,
            &failing,
            VALVE,
            BoostChange::Start {
                duration_minutes: 30,
                temperature: 22.0
            }
        )
        .await
        .unwrap_err()
        .contains("write rejected")
    );
    assert!(valve(&ep).boost.is_none());

    start(&mut ep, repository, 30).await;
    let original = *ep.active_boost(VALVE).unwrap();
    assert_eq!(
        valve(&ep).setpoint,
        Some(20.0),
        "intent must not fabricate a device report"
    );
    tick(&mut ep);
    assert_eq!(ep.world.trvs[VALVE].target.owner(), Some(Owner::WebUI));
    assert_eq!(ep.world.trvs[VALVE].target_setpoint(), Some(22.0));
    clk.advance(Duration::from_secs(600));
    assert!(
        change_valve_boost(
            &mut ep,
            repository,
            VALVE,
            BoostChange::Start {
                duration_minutes: 60,
                temperature: 22.0
            }
        )
        .await
        .unwrap_err()
        .contains("already active")
    );
    change_valve_boost(
        &mut ep,
        repository,
        VALVE,
        BoostChange::Target { temperature: 23.5 },
    )
    .await
    .unwrap();
    assert_eq!(
        ep.active_boost(VALVE).unwrap().ends_at_epoch_ms,
        original.ends_at_epoch_ms
    );
    assert_eq!(valve(&ep).boost.unwrap().remaining_ms, 20 * 60_000);
    for change in [
        BoostChange::Target { temperature: 24.0 },
        BoostChange::Cancel,
    ] {
        assert!(
            change_valve_boost(&mut ep, &failing, VALVE, change)
                .await
                .unwrap_err()
                .contains("write rejected")
        );
        assert_eq!(ep.active_boost(VALVE).unwrap().temperature, 23.5);
        assert_eq!(
            repository.load().await.unwrap().boosts[VALVE].temperature,
            23.5
        );
    }
    let mut restored = EventProcessor::new(
        Arc::new(Topology::build(&cfg).unwrap()),
        clk.clone(),
        cfg.defaults.clone(),
        None,
    );
    restored.restore_settings(repository.load().await.unwrap());
    assert_eq!(valve(&restored).boost.unwrap().remaining_ms, 20 * 60_000);
    clk.advance(Duration::from_secs(20 * 60));
    assert!(valve(&restored).boost.is_none());
    assert!(
        change_valve_boost(
            &mut restored,
            repository,
            VALVE,
            BoostChange::Target { temperature: 25.0 }
        )
        .await
        .unwrap_err()
        .contains("no longer active")
    );
    restored.restore_settings(repository.load().await.unwrap());
    assert!(
        valve(&restored).boost.is_none(),
        "restart must not renew expired boosts"
    );
    tick(&mut ep);
    assert_eq!(ep.world.trvs[VALVE].target_setpoint(), Some(20.0));
    assert_eq!(ep.world.trvs[VALVE].target.owner(), Some(Owner::Schedule));
    for _ in 0..2 {
        change_valve_boost(&mut ep, repository, VALVE, BoostChange::Cancel)
            .await
            .unwrap();
    }
    assert!(repository.load().await.unwrap().boosts.is_empty());
}

#[tokio::test]
async fn boost_contract_memory() {
    boost_contract(&MemorySettings::default()).await;
}

#[tokio::test]
async fn boost_contract_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    boost_contract(
        &SqliteSettings::open(&dir.path().join("settings.db"))
            .await
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn boost_survives_database_reopen_and_cancellation_is_durable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.db");
    let cfg = simple_config();
    let (mut ep, clk) = setup(&cfg);
    let repository = SqliteSettings::open(&path).await.unwrap();
    start(&mut ep, &repository, 90).await;
    let original = valve(&ep).boost.unwrap();
    drop(repository);
    clk.advance(Duration::from_secs(60 * 60));
    let repository = SqliteSettings::open(&path).await.unwrap();
    let mut restarted = EventProcessor::new(
        Arc::new(Topology::build(&cfg).unwrap()),
        clk.clone(),
        cfg.defaults,
        None,
    );
    restarted.restore_settings(repository.load().await.unwrap());
    let restored = valve(&restarted).boost.unwrap();
    assert_eq!(restored.ends_at_epoch_ms, original.ends_at_epoch_ms);
    assert_eq!(restored.remaining_ms, 30 * 60_000);
    change_valve_boost(&mut restarted, &repository, VALVE, BoostChange::Cancel)
        .await
        .unwrap();
    drop(repository);
    assert!(
        SqliteSettings::open(&path)
            .await
            .unwrap()
            .load()
            .await
            .unwrap()
            .boosts
            .is_empty()
    );
}

#[tokio::test]
async fn cancel_and_expiry_resume_the_current_schedule() {
    for cancel in [true, false] {
        let mut cfg = simple_config();
        cfg.heating
            .as_mut()
            .unwrap()
            .schedules
            .get_mut("bath-sched")
            .unwrap()
            .days = Weekday::ALL
            .into_iter()
            .map(|day| (day, two_period_day(20.0, 16.0)))
            .collect();
        let (mut ep, clk) = setup(&cfg);
        let repository = MemorySettings::default();
        start(&mut ep, &repository, 30).await;
        tick(&mut ep);
        assert_eq!(ep.world.trvs[VALVE].target_setpoint(), Some(22.0));
        clk.set_hour(23);
        if cancel {
            change_valve_boost(&mut ep, &repository, VALVE, BoostChange::Cancel)
                .await
                .unwrap();
        } else {
            clk.advance(Duration::from_secs(30 * 60));
        }
        tick(&mut ep);
        assert_eq!(ep.world.trvs[VALVE].target_setpoint(), Some(16.0));
        assert_eq!(ep.world.trvs[VALVE].target.owner(), Some(Owner::Schedule));
        assert!(valve(&ep).boost.is_none());
    }
}

#[tokio::test]
async fn boost_overrides_suppression_but_requires_confirmed_real_demand_and_preserves_pump_limits()
{
    for sonoff in [false, true] {
        let mut cfg = simple_config();
        if sonoff {
            cfg.devices.insert(VALVE.into(), trv_dev_sonoff("0xaa"));
        }
        let (mut ep, clk) = setup(&cfg);
        let repository = MemorySettings::default();
        ep.set_heat_demand_enabled(VALVE, false).unwrap();
        ep.pump_off_since = Some(clk.now());
        start(&mut ep, &repository, 30).await;
        let pump_on = |ep: &EventProcessor, effects: Vec<Effect>| {
            effects
                .iter()
                .any(|a| a.target_name(ep) == "wt-bath" && a.payload_json(ep).contains("ON"))
        };
        let effects = tick(&mut ep);
        assert!(!pump_on(&ep, effects), "boost alone is not reported demand");
        let report = |ep: &mut EventProcessor, setpoint| {
            ep.handle_event(Event::TrvState {
                device: VALVE.into(),
                local_temperature: Some(18.0),
                pi_heating_demand: if sonoff { None } else { Some(50) },
                running_state: Some("heat".into()),
                occupied_heating_setpoint: Some(setpoint),
                operating_mode: None,
                system_mode: None,
                battery: None,
                ts: clk.now(),
            });
        };
        report(&mut ep, 20.0);
        let effects = tick(&mut ep);
        assert!(
            !pump_on(&ep, effects),
            "unconfirmed boost must not start the pump"
        );
        report(&mut ep, 22.0);
        let effects = tick(&mut ep);
        assert!(!pump_on(&ep, effects), "minimum pause still applies");
        clk.advance(Duration::from_secs(60));
        let effects = tick(&mut ep);
        assert!(pump_on(&ep, effects));
        assert!(
            !valve(&ep).heat_demand_enabled,
            "boost must not change the saved suppression flag"
        );
        echo_relay(&mut ep, "wt-bath", true, &clk);
        change_valve_boost(&mut ep, &repository, VALVE, BoostChange::Cancel)
            .await
            .unwrap();
        let effects = tick(&mut ep);
        assert!(
            !effects
                .iter()
                .any(|a| a.target_name(&ep) == "wt-bath" && a.payload_json(&ep).contains("OFF"))
        );
        assert!(
            valve(&ep).forced,
            "minimum-cycle flow protection remains active after cancellation"
        );
        clk.advance(Duration::from_secs(120));
        assert!(
            tick(&mut ep)
                .iter()
                .any(|a| a.target_name(&ep) == "wt-bath" && a.payload_json(&ep).contains("OFF"))
        );
        assert!(!ep.effective_heat_demand_enabled(VALVE));
    }
}

#[tokio::test]
async fn open_window_protection_takes_priority_and_boost_resumes_after_hold() {
    let mut cfg = simple_config();
    cfg.heating.as_mut().unwrap().open_window = OpenWindowProtection {
        detection_minutes: 1,
        inhibit_minutes: 2,
    };
    let (mut ep, clk) = setup(&cfg);
    start(&mut ep, &MemorySettings::default(), 30).await;
    tick(&mut ep);
    send_trv_demand(&mut ep, VALVE, 18.0, 50, "heat", 22.0, &clk);
    tick(&mut ep);
    echo_relay(&mut ep, "wt-bath", true, &clk);
    clk.advance(Duration::from_secs(5));
    send_trv_demand(&mut ep, VALVE, 18.0, 50, "heat", 22.0, &clk);
    clk.advance(Duration::from_secs(65));
    send_trv_demand(&mut ep, VALVE, 18.0, 50, "heat", 22.0, &clk);
    tick(&mut ep);
    assert!(valve(&ep).inhibited);
    assert_eq!(valve(&ep).boost.unwrap().temperature, 22.0);
    clk.advance(Duration::from_secs(121));
    tick(&mut ep);
    assert!(!valve(&ep).inhibited);
    assert_eq!(ep.world.trvs[VALVE].target_setpoint(), Some(22.0));
}

#[tokio::test]
async fn pressure_group_takes_priority_over_an_independent_boost_then_restores_it() {
    let mut cfg = simple_config();
    cfg.devices.insert("second".into(), trv_dev("0xbb"));
    let heating = cfg.heating.as_mut().unwrap();
    heating.zones[0].trvs.push(ZoneTrv {
        device: "second".into(),
        schedule: "bath-sched".into(),
    });
    heating.pressure_groups.push(PressureGroup {
        name: "pair".into(),
        trvs: vec![VALVE.into(), "second".into()],
    });
    let (mut ep, clk) = setup(&cfg);
    let repository = MemorySettings::default();
    ep.set_heat_demand_enabled(VALVE, false).unwrap();
    start(&mut ep, &repository, 30).await;
    change_valve_boost(
        &mut ep,
        &repository,
        "second",
        BoostChange::Start {
            duration_minutes: 60,
            temperature: 23.5,
        },
    )
    .await
    .unwrap();
    tick(&mut ep);
    send_trv_demand(&mut ep, VALVE, 18.0, 50, "heat", 22.0, &clk);
    send_trv_demand(&mut ep, "second", 24.0, 0, "idle", 23.5, &clk);
    tick(&mut ep);
    echo_relay(&mut ep, "wt-bath", true, &clk);
    tick(&mut ep);
    assert!(ep.world.trvs["second"].is_forced_open());
    assert_eq!(ep.active_boost("second").unwrap().temperature, 23.5);
    clk.advance(Duration::from_secs(200));
    change_valve_boost(&mut ep, &repository, VALVE, BoostChange::Cancel)
        .await
        .unwrap();
    tick(&mut ep);
    assert!(!ep.world.trvs["second"].is_forced_open());
    assert_eq!(ep.world.trvs["second"].target_setpoint(), Some(23.5));
    assert_eq!(ep.world.trvs["second"].target.owner(), Some(Owner::WebUI));
    assert_eq!(ep.boost_remaining_ms("second"), Some((3600 - 200) * 1000));
}

#[tokio::test]
async fn active_boost_does_not_make_stale_demand_usable() {
    let (mut ep, clk) = setup(&simple_config());
    start(&mut ep, &MemorySettings::default(), 60).await;
    tick(&mut ep);
    send_trv_demand(&mut ep, VALVE, 18.0, 50, "heat", 22.0, &clk);
    clk.advance(Duration::from_secs(30 * 60));
    echo_relay(&mut ep, "wt-bath", false, &clk);
    let effects = tick(&mut ep);
    assert!(valve(&ep).boost.is_some());
    assert!(
        !effects
            .iter()
            .any(|a| a.target_name(&ep) == "wt-bath" && a.payload_json(&ep).contains("ON"))
    );
}
