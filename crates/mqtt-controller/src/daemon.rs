//! The runtime daemon: wires the [`MqttBridge`] to the [`EventProcessor`],
//! performs the startup state refresh, then runs the event loop until
//! shutdown.
//!
//! Submodules:
//!
//!   * [`startup`]    — native bridge API state refresh that primes the
//!     world state before normal event processing begins.
//!   * [`event_loop`] — the long-running select loop.
//!   * [`web_bridge`] — WebSocket command + broadcast helpers used by
//!     the event loop when started with a [`WebHandle`].
//!
//! ## Startup state refresh
//!
//! [`MqttBridge::start`] subscribes to bridge MQTT traffic before startup
//! queries the Zigbee2MQTT and Z-Wave WebSocket caches. Cached bulb states
//! derive zone observations through the same handler as live reports:
//! any member ON means ON, every member known OFF means OFF, otherwise
//! the aggregate is unknown. Group reports are not required for this.
//!
//! Seed failures are logged and non-fatal. Queued MQTT events are processed
//! after seeding; subsequent unsolicited reports continue updating state.
//! Startup reads bridge caches, not fresh Zigbee `/get` responses.

use std::sync::Arc;

use anyhow::Context;
use thiserror::Error;

use crate::config::Config;
use crate::effect_dispatch;
use crate::logic::EventProcessor;
use crate::mqtt::{MqttBridge, MqttConfig, MqttError};
use crate::settings::SettingsRepository;
use crate::time::Clock;
use crate::topology::{Topology, TopologyError};
use crate::web::server::WebHandle;

mod event_loop;
mod startup;
mod web_bridge;
mod z2m_seed;
mod zwave_seed;
mod zwave_server;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("topology validation failed: {0}")]
    Topology(#[from] TopologyError),

    #[error("mqtt error: {0}")]
    Mqtt(#[from] MqttError),
}

/// Build and run the daemon. Blocks until shutdown.
///
/// If `web` is `Some`, the event loop serves WebSocket commands and
/// broadcasts state updates / decision logs to connected clients.
/// Both `z2m_ws_url` and `zwave_ws_url` are optional — `None` disables
/// that bridge's startup seed. Useful for integration tests and for
/// deployments without the z2m frontend or `zwave-js-server`.
/// When disabled, the daemon starts with unknown state for that bridge;
/// live publishes from the wildcard MQTT subscription fill it in.
pub async fn run(
    config: Config,
    mqtt: MqttConfig,
    z2m_ws_url: Option<String>,
    zwave_ws_url: Option<String>,
    clock: Arc<dyn Clock>,
    web: Option<WebHandle>,
    settings: impl SettingsRepository,
) -> anyhow::Result<()> {
    let topology = Arc::new(Topology::build(&config).context("topology validation")?);
    let defaults = config.defaults.clone();

    tracing::info!(
        rooms = topology.rooms().count(),
        switches = topology.all_switch_device_names().len(),
        motion_sensors = topology.all_motion_sensor_names().len(),
        groups = topology.all_group_names().len(),
        plugs = topology.all_plug_names().len(),
        trvs = topology.all_trv_names().len(),
        wall_thermostats = topology.all_wall_thermostat_names().len(),
        bindings = topology.bindings().len(),
        heating = topology.heating_config().is_some(),
        "topology built"
    );

    let mut processor = EventProcessor::new(topology.clone(), clock.clone(), defaults, config.location);
    processor.restore_motion_settings(settings.load().await.context("loading motion settings")?);

    tracing::info!(
        host = %mqtt.host,
        port = mqtt.port,
        "connecting to mqtt"
    );
    let (bridge, event_rx) = MqttBridge::start(mqtt, topology.clone(), clock.clone())
        .await
        .context("connecting to mqtt broker")?;

    startup::refresh_state(
        &mut processor,
        &topology,
        &*clock,
        z2m_ws_url.as_deref(),
        zwave_ws_url.as_deref(),
    )
    .await?;
    let mut event_rx = event_rx;

    // Turn off any motion-controlled room that was left on before
    // restart. No cooldown is applied so motion sensors can
    // immediately re-trigger if someone is actually in the room.
    let startup_effects = processor.startup_turn_off_motion_zones(clock.now());
    effect_dispatch::dispatch(&bridge, &topology, &startup_effects).await;
    processor.arm_kill_switches_for_active_plugs(clock.now());

    tracing::info!("startup state refresh complete; entering event loop");

    event_loop::run_event_loop(&mut processor, &bridge, &mut event_rx, web, clock, &settings).await
}
