# MQTT controller dashboard

The dashboard lives in `frontend`: React, TypeScript, Vite and
Zod. Axum serves its static assets alongside `/ws`; there is no browser event log.
The former Leptos/WASM frontend and Trunk build have been removed.

## Development and validation

Use Node.js 24 and the committed npm lockfile. The flake dev shell
(`nix develop`) provides Node.js 24, the Rust toolchain, mold and a Chromium
for the browser tests:

```sh
cd frontend
npm ci
npm run dev
npm test
npm run build
npm run test:browser
```

Vite proxies `/ws` to a controller on `127.0.0.1:8780`. Development controls issue
real commands to that controller. Browser tests instead start their own local
WebSocket server with synthetic devices and serve the built assets from `dist`,
so run `npm run build` first. Playwright's bundled Chromium is an FHS build that
cannot load its shared libraries on NixOS; the dev shell sets
`PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` to its Nix-built Chromium:

```sh
nix develop -c npm run test:browser
```

Outside the dev shell, set `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` to an installed
Chromium, or install Playwright's Chromium with `npx playwright install chromium`
where FHS library paths resolve.

Backend tests use an embedded MQTT broker and temporary databases:

```sh
cargo test -p mqtt-controller -p mqtt-controller-wire --locked
```

The flake builds static assets with `nix/frontend.nix`, runs frontend tests,
and embeds the output in the controller package. To verify the frontend build:

```sh
nix build --no-link .#mqtt-controller-frontend
```

## Controls and state

Light groups expose their scheduled scene IDs and an OFF action. Plugs expose
explicit ON/OFF actions, so a repeated request cannot toggle the wrong way after
an outdated observation. Requested and reported state remain separate. A command
acknowledgement means the controller accepted the action; hardware confirmation
arrives through subsequent MQTT observations. Validation failures, missing
acknowledgements and interrupted connections appear beside the affected control.

`mqtt-controller-wire` defines the Rust protocol; `frontend/src/protocol.ts`
validates received messages in the browser. Update both when changing the wire
contract. Commands carry a request ID and receive a matching `CommandResult`.
They are never queued offline or replayed after reconnection.

Zones with motion sensors also expose a persisted **Motion triggers** switch.
Its acknowledgement means the setting has been saved, not a light command sent;
the switch follows controller snapshots, independently of device confirmation.
The daemon requires `--settings-db PATH` (the NixOS module supplies it).
See [motion rules](motion.md#dashboard-motion-toggle) for cancellation and
overlapping-zone semantics.

Each valve also exposes a persisted **Heat demand** switch. Disabling it excludes
that valve from zone-relay demand and from triggering its pressure group. It does
not change its schedule, reported state or telemetry. Minimum pump-cycle and
pressure protection still apply, so suppression does not guarantee an immediate
relay OFF or a closed valve. This setting uses the same settings database as
motion switches; failed saves leave the previous setting intact.

Each valve has a timed **Boost**: choose 30m, 1h, 1h 30m, 2h, 3h, 4h or 6h,
then press Boost. It starts at **22°C** and replaces the duration/start controls
with a target input and **Cancel boost**. Set the target from 5–30°C in 0.5°C
steps; Enter or leaving the input saves it. Editing does not extend the timer.
The controller saves the target and expiry in its settings database before
acknowledging; restarts restore only the remaining time. Closing the dashboard
does not cancel Boost. Failed saves leave the previous boost intact.

Boost temporarily replaces the schedule and enables this valve's demand even
if its saved Heat demand switch is off. It does not fabricate demand or force
the pump ON: the valve must confirm its setpoint and report heat demand.
Open-window holds, pressure-group protection and minimum pump run/pause times
still take priority. Cancellation or expiry resumes the **current** schedule and
saved demand setting on the next control tick (normally within five seconds).
Flow protection can keep valves open or relays running longer. The browser's
countdown is informational; the controller owns expiry.

**Edit schedule** (right of the Boost controls) opens a weekly editor for that
valve. Pick a day tab, then edit each period's start time and target; the first
period always starts at 00:00 and the last ends at 24:00. **Add period** splits
the last period, **Remove** merges a period into the one before it, and **Copy
… to** applies the shown day to all days, weekdays or the weekend. Each day must
cover 00:00–24:00 without gaps or overlaps; targets must be 5–30°C. **Save
schedule** persists a per-valve override in the settings database and applies
it on the next control tick, including after a restart. The schedule shown on
the valve card summarizes the current day. While an override is active,
**Restore defaults** inside the editor removes it and resumes the schedule from
the deployed configuration.
An active Boost still takes priority until it ends or is cancelled.

### Light and plug automation overrides

Light groups and plugs each have an **Edit schedule** editor of the same
shape. Every override is persisted in the settings database, applies
immediately and survives restarts. **Save schedule** sends only what changed;
**Restore defaults** inside the editor removes every override the editor
covers and returns to the deployed configuration. Failed saves leave the
previous values intact and are reported on the card.

- A light group's editor has **Time slots** and, when the group has daily
  bindings, **Timed actions**. Each slot has a name, start, end, an ordered
  list of scenes (the first is used first) and optional **switch steps**: with
  steps, each press of the group's switch turns the listed lights on with the
  step's scene and every other member off, instead of cycling the whole group.
  Slots can be renamed, **Split** at their midpoint, **Add**ed and **Remove**d;
  the editor checks fixed-time schedules for gaps and overlaps before saving.
  Times use the configuration syntax (`HH:MM`, `sunrise`/`sunset` with an
  optional `±HH:MM` offset, or `min(a, b)`/`max(a, b)`); sun-relative times
  require a configured location and are validated by the controller. Scene
  contents are provisioned into Zigbee groups and cannot be edited here. A
  saved schedule replaces the deployed slots and switch steps together and
  forgets the switch's current step selection.
- **Edit motion schedule**, under *Lights & automation*, edits a motion rule's
  slot boundaries and scenes. Slot names stay as deployed because the rule's
  targets refer to them, and every slot needs at least one scene. A running
  motion session keeps its slot's target.
- **Timed actions** in a light group's editor are the daily `at` bindings
  acting on the group, including those that turn off all groups. Each
  binding's time can be changed; `24:00` is rejected. The bindings themselves
  come from the deployed configuration.
- A plug's editor edits the plug's whole schedule: any number of daily
  **timed actions** (turn on, turn off or toggle at a time expression) and an
  optional **power-off rule** (a positive watt threshold and a holdoff of at
  least one second, only for plugs that report power). Actions and the rule
  can be added and removed even when the deployed configuration has none.
  Saving stores the schedule per plug and supersedes every deployed `at` and
  `power_below` binding of that plug until **Restore defaults**. A running
  idle timer restarts from the plug's current reading when the rule changes.

The *Lights & automation* and *Automation & device* sections show the effective
timed actions and power-off rules read-only. If a new deployment removes or
reshapes something an override refers to, the controller logs a warning at
startup, ignores the override and uses the deployed values. The database row
remains until it is saved again or reset. Light-group overrides saved before
switch steps were part of them, and per-binding kill-switch overrides from
before plug schedules existed, are dropped with a warning when the controller
first opens the database.

## Energy views

**Energy → Plugs** and **Energy → Heating** show compact chart lists grouped by
room or heating zone. Hover readings appear in floating popups; clicking a chart
still opens its recorded-values table.

On mobile, the Lights/Plugs/Heating tabs open scrollable navigation popups.
Plugs and Heating put their Energy view first, followed by the device view and
room/zone shortcuts. Shortcuts retain the current view when its category matches,
otherwise switch to that category's device view; selecting one clears search.
Escape, the close button or tapping outside dismisses a popup. Desktop sidebar
navigation is unchanged.

A plug catalog entry may set `exclude_from_totals: true` to exclude its estimated
energy from every room and overall total while retaining its individual chart.
In the private Nix device configuration this is `excludeFromTotals = true`.
Only the primary and reserve feeds count towards rack totals; their downstream
plugs are excluded.

Heating includes observed relay ON/OFF history and each zone's ON duration for
the last 24 hours. **Zone relay-hours** sums durations, counting simultaneous
zones separately. **Any relay ON** counts overlapping intervals only once.
Requested relay state never counts as reported runtime. Unknown or stale
endpoints and sample gaps longer than 90 seconds are excluded, with usable
coverage displayed alongside each chart.

`heating.energy_meter` selects a catalog device with `kind: "power-meter"`.
This read-only device accepts Zigbee2MQTT `power` (W) and cumulative consumed
`energy` (kWh), with independent freshness. The heat-pump chart uses observed
power; consumption uses differences in the energy counter, not relay runtime
or an assumed pump rating. Counter differences span reporting gaps; decreases
are treated as resets and the affected interval is excluded and reported.
Meter readings become stale after 65 minutes to accommodate its hourly reports.

## Telemetry history

With the web interface enabled, the service records valve, plug, zone-relay and
heat-pump snapshots per minute in `/var/lib/mqtt-controller/heating-history.db`. Standalone
invocations must provide `--web-history-db PATH` alongside `--web-port` and
`--web-assets-dir`. Database initialization failures prevent startup; subsequent
sampling failures are logged and shown with history results.

History survives restarts and retains the last 24 hours. Recording begins after
deployment; existing audit entries cannot reconstruct past temperatures. Each
sample retains observed temperature, reported setpoint, requested target, demand,
battery, freshness and last observation time. Repeated samples within a minute
replace that minute's row. This is sampled history: short changes between samples
may not appear. Plug samples retain observed power and meter freshness separately
from relay state: a meter-only report cannot confirm an on/off command, and a
relay-only report cannot refresh the power reading. The displayed
24-hour energy consumption is a trapezoidal estimate over adjacent fresh power
samples; gaps longer than 90 seconds are excluded. The estimate shows the duration
covered by usable intervals without extrapolating across gaps. When no usable
interval exists, consumption is shown as unknown; measured zero consumption is
shown as `0.00 kWh`. Charts leave gaps for missing samples and stale/unknown
readings, and do not interpolate setpoint changes.
Clicking either chart selects the nearest sample and opens the recorded-values
table at that row. Requested non-temperature modes remain available in that
table.

## Connection recovery

Each socket moves through NEW, ALIVE, STALE and DEAD. A matching nonce and echoed
timestamp establish liveness; an open transport alone does not. The manager uses
handshake deadlines, per-ping deadlines, a stale grace period with an overlapping
replacement, bounded jittered exponential backoff and a manual retry after the
attempt ceiling or a permanent protocol error. Every promotion requests a fresh
snapshot before enabling controls. Lost server broadcast updates close the
connection so the browser must resynchronize.

Visibility, network changes, page freeze/resume, BFCache restoration and detected
timer gaps trigger verification or recovery. Teardown prevents new connections.
The indicator has text, a countdown ring, a title status and an expandable view
of connection IDs, RTT windows, heartbeat timeouts and retry/close details.

The server keeps ordered command/database work in a bounded worker separate from
its heartbeat reader. Protocol pongs must match the current or previous nonce.
Before checking the heartbeat deadline, a short Tokio timer defers the check so
buffered I/O can run after a scheduler pause; Node's `setImmediate` is not
applicable to this Rust server.

Two deliberate limits relative to `resilient-ws-ui`: the user requested no event
log, and heartbeats run on the browser's main thread. Hidden tabs may throttle or
suspend timers; the dashboard verifies/replaces connections when visible again
instead of promising continuous background connectivity. Reconnection starts a
fresh session; uncertain commands require checking reported state before retrying.
