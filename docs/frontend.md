# MQTT controller dashboard

The dashboard lives in `frontend`: React, TypeScript, Vite and
Zod. Axum serves its static assets alongside `/ws`; there is no browser event log.
The former Leptos/WASM frontend and Trunk build have been removed.

## Development and validation

Use Node.js 24 and the committed npm lockfile:

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
WebSocket server with synthetic devices. Set
`PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` to use an installed Chromium; otherwise
install Playwright's Chromium with `npx playwright install chromium`.

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
