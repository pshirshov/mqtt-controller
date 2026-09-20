# Motion lighting rules

Rooms define Zigbee groups and their provisioned scenes. Independent `motionRules`
connect sensors to lighting targets selected by the rule's scene schedule.

For example, with `bathroomScenes` containing the existing four slots:

```nix
motionRules = [ {
  name = "bathroom-motion";
  sensors = [ "hue-ms-bathroom" ];
  scenes = bathroomScenes;
  targetsBySlot = {
    day = { group = "bathroom"; };
    evening = { lights = [ "hue-l-bathroom-wall/11" ]; };
    night = { lights = [ "hue-l-bathroom-wall/11" ]; };
    morning = { lights = [ "hue-l-bathroom-wall/11" ]; };
  };
} ];
```

`group` references a room's logical name. `lights` contains device/endpoint
references already present in configured groups. This adds no Zigbee groups or
stored scenes. Group targets recall the first scene of the active slot; individual
targets receive that scene's ON state, brightness, color temperature and transition
directly. Group-target scenes must match the group's provisioned scene definitions.

`defaults.motion` supplies `mode`, `offCooldownSeconds`, `offTransitionSeconds` and
`maxIlluminance`; each rule can override them. A null illuminance threshold disables
the gate. Sensor reporting settings, including occupancy timeout, remain in the
device catalog. The JSON equivalents use snake_case (`motion_rules`,
`targets_by_slot`, etc.). Old room motion fields and sensor illuminance gates must
be migrated to rules.

Each occupancy session retains its selected targets until all sensors are clear
or stale. Crossing a slot boundary changes the next session. Repeated occupied
reports refresh sensor freshness without replaying activation. The illuminance
gate and cooldown apply when starting a session; cooldown begins after an
automatic OFF command. Sensors silent for five minutes count as stale.

- `on-off` turns on eligible lights and turns off the lights it still owns on vacancy.
- `on-only` turns on eligible lights and leaves them on after vacancy.
- `off-only` claims the selected lights without turning them on, then turns them
  off on vacancy. Manual controls retain this claim while occupancy remains fresh.

Manual group commands take over affected lights from `on-off` motion. This applies
to overlapping groups and brightness controls. A motion rule does not take over
lights already on. Vacancy never turns off lights that manual commands took over.

Rules may share sensors when their possible lighting targets are disjoint.
Overlapping rule targets, missing slot targets, unknown endpoints and conflicting
group scenes are rejected during configuration validation. Motion-controlled
devices currently require one configured endpoint because telemetry is tracked
per device.

On startup, the existing reset policy applies to the union of each rule's targets:
lit targets are switched off without arming cooldown. `on-only` is excluded;
`off-only` with seeded fresh occupancy adopts the current slot's targets instead.
The dashboard shows rule schedules, active session targets, cooldown, and each
light's target ownership and confirmation status.

## Dashboard motion toggle

Every zone with motion sensors has a **Motion triggers** switch, enabled by
default. Disabling it prevents motion-driven ON and OFF commands for that zone's
lights, including vacancy, stale-sensor cleanup and the startup reset. Active
claims are released without changing the lights. Sensor readings and manual
controls remain available. Re-enabling waits for the next occupancy activation;
it does not immediately replay an occupied sensor's last report.

For overlapping zones, disabling any containing zone suppresses motion for the
shared lights. Other lights in the same rule remain controlled; group commands
are replaced by individual commands when necessary.

Settings are saved on the controller, not in browser storage, and survive both
browser reloads and daemon restarts. The daemon requires `--settings-db PATH`;
the NixOS module supplies `/var/lib/mqtt-controller/settings.db`, even with the
dashboard disabled. Settings load before startup automation. A failed load
prevents startup; a failed save rejects the command and leaves runtime state
unchanged. Renaming a zone creates a new setting (enabled by default).

Validate a rendered configuration against the Rust schema and topology:

```sh
MQTT_CONTROLLER_CONFIG=/path/to/config.json cargo test -p mqtt-controller \
  --test motion_rules rendered_configuration_builds_topology -- --ignored
```
