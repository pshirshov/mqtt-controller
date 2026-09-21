import { Fragment, useEffect, useState, useSyncExternalStore } from 'react';
import { ConnectionIndicator, healthLabel } from './ConnectionIndicator';
import { HistoryChart, PowerHistoryChart } from './HistoryChart';
import { age, duration, label, temperature, valveTarget } from './format';
import type { ActualMeta, HeatingZone, Light, Plug, PlugPowerHistory, Room, TargetMeta, Valve } from './protocol';
import { DashboardClient, type CommandStatus, type HistoryStatus, type Timed } from './store';

type Page = 'lights' | 'plugs' | 'heating';
const PAGES: Page[] = ['lights', 'plugs', 'heating'];
function currentPage(): Page {
  const hash = window.location.hash.slice(1);
  return hash === 'plugs' || hash === 'heating' ? hash : 'lights';
}

function plugRoom(plug: Timed<Plug>): string { return plug.value.room ?? 'unassigned'; }
function totalPlugEnergyKwh(plugs: Timed<Plug>[], histories: ReadonlyMap<string, HistoryStatus<PlugPowerHistory>>): number | null {
  let total = 0;
  let hasEstimate = false;
  for (const plug of plugs) {
    const history = histories.get(plug.value.device);
    if (history === undefined || history.data === null || history.data.estimated_energy_kwh === null) continue;
    hasEstimate = true;
    total += history.data.estimated_energy_kwh;
  }
  return hasEstimate ? total : null;
}
function sectionId(name: string): string { return `section-${encodeURIComponent(name)}`; }
function scrollToSection(name: string): void {
  const section = document.getElementById(sectionId(name));
  if (section === null) throw new Error(`Missing dashboard section: ${name}`);
  section.scrollIntoView({
    block: 'start',
    behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth',
  });
}

export function App({ client }: { client: DashboardClient }) {
  const state = useSyncExternalStore(client.subscribe, client.getSnapshot);
  const [page, setPage] = useState<Page>(currentPage);
  const [search, setSearch] = useState('');
  useEffect(() => {
    const change = () => { setPage(currentPage()); setSearch(''); };
    window.addEventListener('hashchange', change);
    return () => window.removeEventListener('hashchange', change);
  }, []);
  useEffect(() => {
    if (!state.ready || page === 'lights') return;
    const devices = page === 'heating'
      ? state.heating.flatMap(zone => zone.value.trvs.map(valve => valve.device))
      : state.plugs.map(plug => plug.value.device);
    const refresh = () => { for (const device of devices) {
      if (page === 'heating') client.loadHistory(device);
      else client.loadPlugPowerHistory(device);
    } };
    refresh();
    const timer = window.setInterval(refresh, 60_000);
    return () => window.clearInterval(timer);
    // Device membership changes only with a new full snapshot/deployment.
  }, [client, page, state.ready]);
  const query = search.trim().toLowerCase();
  const matches = (...values: string[]) => values.some(value => label(value).toLowerCase().includes(query) || value.toLowerCase().includes(query));
  const rooms = state.rooms.filter(room => matches(room.value.name, room.value.room, ...room.value.lights.map(light => light.device)));
  const plugs = state.plugs.filter(plug => matches(plug.value.device, plug.value.display_name ?? '', plug.value.room ?? ''));
  const zones = state.heating.filter(zone => matches(zone.value.name, ...zone.value.trvs.map(valve => valve.device)));
  const sections = [...new Set(page === 'lights' ? rooms.map(room => room.value.room)
    : page === 'plugs' ? plugs.map(plugRoom) : zones.map(zone => zone.value.name))];
  const onRooms = state.rooms.filter(room => room.value.actual_value === 'on').length;
  const onPlugs = state.plugs.filter(plug => plug.value.actual_value != null && plug.value.actual_value.on).length;
  const plugEnergyKwh = totalPlugEnergyKwh(state.plugs, state.plugHistories);
  const demandZones = state.heating.filter(zone => zone.value.target_value === 'heating').length;
  const totals = { lights: state.rooms.length, plugs: state.plugs.length, heating: state.heating.length };
  const subtitles = {
    lights: `${onRooms} groups on · ${state.lights.length} lights`,
    plugs: `${onPlugs} plugs on · ${state.plugs.length} total · ${plugEnergyKwh === null ? '—' : plugEnergyKwh.toFixed(2)} kWh last 24h`,
    heating: `${demandZones} zones requesting heat · ${state.heating.flatMap(zone => zone.value.trvs).length} valves`,
  };
  return <div className="app-shell">
    <aside className="sidebar">
      <a className="brand" href="#lights"><span className="brand-mark">h<span>.</span></span><span>HOME<small>MQTT controller</small></span></a>
      <span className="nav-caption">CONTROLS</span>
      <nav aria-label="Main navigation">{PAGES.map(item => <Fragment key={item}><a href={`#${item}`} aria-current={page === item ? 'page' : undefined} className={page === item ? 'nav-link selected' : 'nav-link'}>
        <Icon kind={item} /><span>{label(item)}</span><small>{totals[item]}</small>
      </a>{item === page && sections.length > 0 && <div className="section-nav" role="group" aria-label={`${label(item)} ${item === 'heating' ? 'zones' : 'rooms'}`}>
        {sections.map(name => <button key={name} type="button" aria-controls={sectionId(name)} onClick={() => scrollToSection(name)}>{label(name)}</button>)}
      </div>}</Fragment>)}</nav>
      <div className="sidebar-foot"><span className="small-dot" /> Your home, at a glance<small>Live device state & controls</small></div>
    </aside>
    <main>
      <header className="page-header"><div><p className="eyebrow">HOME CONTROLS</p><h1>{label(page)}</h1><p className="page-subtitle">{state.receivedAt === null ? 'Connecting to your home…' : subtitles[page]}</p></div>
        <ConnectionIndicator health={state.health} retry={() => client.connection.retry()} />
      </header>
      {!state.ready && <div className="connection-banner" role="status"><span>{healthLabel(state.health)}</span> · {state.receivedAt === null ? 'Waiting for the controller. Controls will be available once state is received.' : 'Showing last known device state. Controls are paused until the connection is verified.'}</div>}
      <div className="toolbar"><div><span className="section-kicker">{page === 'heating' ? 'HEATING ZONES' : page === 'lights' ? 'LIGHT GROUPS' : 'SMART PLUGS'}</span><span className="toolbar-count">{totals[page]}</span></div>
        <label className="search"><span aria-hidden="true">⌕</span><input aria-label={`Search ${page}`} placeholder={page === 'heating' ? 'Find a zone or valve…' : `Find ${page === 'lights' ? 'a group or light' : 'a plug'}…`} value={search} onChange={event => setSearch(event.target.value)} type="search" /></label>
      </div>
      {state.receivedAt === null ? <div className="card-grid" aria-label="Loading devices">{[1, 2, 3, 4, 5, 6].map(index => <div key={index} className="skeleton" />)}</div>
        : page === 'lights' ? <RoomGroups items={rooms} room={item => item.value.room} summary={null} render={item => <RoomCard key={item.value.name} room={item} lights={state.lights} live={state.ready} status={state.commands.get(`room:${item.value.name}`)} client={client} />} />
        : page === 'plugs' ? <RoomGroups items={plugs} room={plugRoom} summary={name => {
          const energyKwh = totalPlugEnergyKwh(state.plugs.filter(plug => plugRoom(plug) === name), state.plugHistories);
          return `${energyKwh === null ? '—' : energyKwh.toFixed(2)} kWh last 24h`;
        }} render={item => <PlugCard key={item.value.device} plug={item} live={state.ready} history={state.plugHistories.get(item.value.device)} status={state.commands.get(`plug:${item.value.device}`)} client={client} />} />
        : <div className="heating-zones">{zones.map(zone => <HeatingCard key={zone.value.name} zone={zone} live={state.ready} histories={state.histories} client={client} />)}</div>}
      {state.receivedAt !== null && (page === 'lights' ? rooms.length : page === 'plugs' ? plugs.length : zones.length) === 0 && <div className="empty-state"><Icon kind={page} /><h2>{query === '' ? `No ${page} configured` : 'Nothing matches this search'}</h2>{query !== '' && <button className="button" onClick={() => setSearch('')}>Clear search</button>}</div>}
      <footer className="page-footer">Requested state is what the controller wants. Reported state is what the device last confirmed.</footer>
    </main>
  </div>;
}

function RoomGroups<T>({ items, room, summary, render }: { items: T[]; room: (item: T) => string; summary: ((name: string) => string) | null; render: (item: T) => React.ReactNode }) {
  const groups = new Map<string, T[]>();
  for (const item of items) {
    const name = room(item);
    groups.set(name, [...(groups.get(name) ?? []), item]);
  }
  return <div className="room-groups">{[...groups].map(([name, members]) => <section className="room-group" id={sectionId(name)} aria-label={label(name)} key={name}>
    <div className="room-group-heading"><h2>{label(name)}</h2>{summary !== null && <span>· {summary(name)}</span>}</div>
    <div className="card-grid">{members.map(render)}</div>
  </section>)}</div>;
}

function Icon({ kind }: { kind: Page }) {
  return <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    {kind === 'lights' ? <><path d="M9 18h6M10 21h4M8 14c-5-5-1-11 4-11s9 6 4 11l-1 2H9z" /><path d="M12 1v1M3 5l1 1M20 5l1-1M1 12h2M21 12h2" /></>
      : kind === 'plugs' ? <><path d="M8 3v5M16 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v5" /></>
      : <><rect x="4" y="5" width="4" height="15" rx="2" /><rect x="10" y="5" width="4" height="15" rx="2" /><rect x="16" y="5" width="4" height="15" rx="2" /><path d="M2 10h2M20 15h2M6 2v1M12 2v1M18 2v1" /></>}
  </svg>;
}

function Badge({ children, tone }: { children: React.ReactNode; tone: string }) {
  return <span className={`badge ${tone}`}>{children}</span>;
}
function Freshness({ actual, receivedAt, live }: { actual: ActualMeta | null | undefined; receivedAt: number; live: boolean }) {
  const freshness = actual == null ? 'unknown' : actual.freshness;
  return <div className="freshness"><span className={`freshness-dot ${live ? freshness : 'stale'}`} /><span>{!live ? 'Last known' : freshness === 'fresh' ? 'Fresh' : label(freshness)}</span><span className="freshness-age">{age(actual, receivedAt, Date.now())}</span></div>;
}
function StatePair({ requested, reported, target }: { requested: string; reported: string | null; target: TargetMeta | null | undefined }) {
  return <div className="state-pair"><div><span className="field-label">REQUESTED</span><strong>{requested}</strong><small>{target == null || target.phase === 'unset' ? 'No active target' : `${label(target.owner)} · ${label(target.phase)}`}</small></div><div><span className="field-label">REPORTED</span><strong>{reported ?? 'Unknown'}</strong><small>{reported === null ? 'Awaiting device reports' : 'Last reported state'}</small></div></div>;
}
function Feedback({ status }: { status: CommandStatus | undefined }) {
  if (status === undefined) return null;
  return <p className={`command-feedback ${status.state}`} role={status.state === 'error' ? 'alert' : 'status'}>{status.message}</p>;
}

function RoomCard({ room, lights, live, status, client }: { room: Timed<Room>; lights: Timed<Light>[]; live: boolean; status: CommandStatus | undefined; client: DashboardClient }) {
  const value = room.value;
  const isOn = value.actual_value === 'on';
  const unknown = value.actual_value == null;
  const busy = status !== undefined && status.state === 'pending';
  const requested = value.target_value == null ? '—' : value.target_value.kind === 'off' ? 'Off' : `Scene ${value.target_value.scene_id}`;
  const key = `room:${value.name}`;
  return <article className={`device-card ${isOn ? 'is-on' : ''}`} aria-label={label(value.name)}>
    <div className="card-heading"><span className={`device-icon ${isOn ? 'lit' : ''}`}><Icon kind="lights" /></span><div><h2>{label(value.name)}</h2><p>{value.lights.length} {value.lights.length === 1 ? 'light' : 'lights'}{value.active_slot !== null && ` · ${label(value.active_slot)}`}</p></div><Badge tone={unknown ? 'neutral' : isOn ? 'warm' : 'neutral'}>{unknown ? 'Unknown' : isOn ? 'On' : 'Off'}</Badge></div>
    <StatePair requested={requested} reported={unknown ? null : isOn ? 'On' : 'Off'} target={value.target} />
    <div className="room-controls" aria-label={`${label(value.name)} controls`}>
      <div className="scene-buttons">{value.scene_ids.map(id => <button key={id} className={`button scene ${value.target_value != null && value.target_value.kind === 'on' && value.target_value.scene_id === id ? 'chosen' : ''}`} disabled={!live || busy} aria-label={`Recall scene ${id} in ${label(value.name)}`} onClick={() => client.command(key, { kind: 'RecallScene', room: value.name, scene_id: id })}>Scene {id}</button>)}</div>
      <button className="button off-button" disabled={!live || busy} aria-label={`Turn off ${label(value.name)}`} onClick={() => client.command(key, { kind: 'SetRoomOff', room: value.name })}><span aria-hidden="true">⏻</span> Off</button>
    </div>
    <Feedback status={status} />
    {value.motion_rules.length > 0 && <label className="motion-toggle">
      <span>Motion triggers</span>
      <input type="checkbox" role="switch" aria-label={`Motion triggers in ${label(value.name)}`}
        checked={value.motion_enabled} disabled={!live || busy}
        onChange={event => client.command(key, { kind: 'SetMotionEnabled', room: value.name, enabled: event.target.checked })} />
      <span>{value.motion_enabled ? 'On' : 'Off'}</span>
    </label>}
    <Freshness actual={value.actual} receivedAt={room.receivedAt} live={live} />
    <details className="device-details"><summary>Lights & automation <span>{value.motion_rules.length > 0 ? value.motion_enabled ? 'Motion enabled' : 'Motion disabled' : `${value.lights.length} members`}</span></summary>
      <div className="member-list">{value.lights.map(member => {
        const item = lights.find(light => light.value.device === member.device);
        const actual = item === undefined ? null : item.value.actual_value;
        const target = item === undefined ? null : item.value.target;
        return <div className="member" key={member.device}><span className={`small-dot ${actual == null ? 'unknown' : actual.on ? 'on' : 'off'}`} /><span title={member.device}>{label(member.device)}<small>{target == null || target.phase === 'unset' ? 'No target' : `${label(target.owner)} · ${label(target.phase)}`}</small></span><strong>{actual == null ? 'Unknown' : !actual.on ? 'Off' : actual.brightness == null ? 'On' : `${Math.round(actual.brightness / 254 * 100)}%`}</strong></div>;
      })}</div>
      {value.motion_rules.map(rule => <div className="automation" key={rule.name}><div className="detail-heading"><strong>{label(rule.name)}</strong><Badge tone="cool">{rule.mode}</Badge></div>
        <p>{rule.active_slot === null ? 'No active slot' : label(rule.active_slot)} → {rule.targets.map(label).join(', ') || 'No targets'}</p>
        {rule.session_targets.length > 0 && <p>Active session: {rule.session_targets.map(label).join(', ')}</p>}
        {rule.cooldown_remaining_secs != null && <p>Cooldown: {Math.max(0, Math.ceil(rule.cooldown_remaining_secs - (Date.now() - room.receivedAt) / 1000))}s</p>}
        {rule.max_illuminance != null && <p>Activate below {rule.max_illuminance} lx</p>}
        {rule.sensors.map(sensor => <div className="sensor" key={sensor.device}>
          <span>{label(sensor.device)}</span><strong>{sensor.occupied == null ? 'Unknown' : sensor.occupied ? 'Motion' : 'Clear'}</strong>
          <small>{sensor.illuminance == null ? '—' : `${sensor.illuminance} lx`} · {sensor.freshness}</small>
          <small className="motion-event">{sensor.last_event === null ? 'No live motion report since restart' : <>
            Last event: <time dateTime={new Date(sensor.last_event.timestamp_epoch_ms).toISOString()}
              title={new Date(sensor.last_event.timestamp_epoch_ms).toLocaleString()}>
              {new Date(sensor.last_event.timestamp_epoch_ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })}
            </time> · {label(sensor.last_event.kind)}
          </>}</small>
        </div>)}
      </div>)}
      {value.switches.length > 0 && <p className="switches">Switches: {value.switches.map(item => label(item.device)).join(', ')}</p>}
      <small className="device-id">{value.group_name}</small>
    </details>
  </article>;
}

function PlugCard({ plug, live, history, status, client }: { plug: Timed<Plug>; live: boolean; history: HistoryStatus<PlugPowerHistory> | undefined; status: CommandStatus | undefined; client: DashboardClient }) {
  const value = plug.value;
  const name = value.display_name ?? label(value.device);
  const actual = value.actual_value;
  const busy = status !== undefined && status.state === 'pending';
  const power = value.power_watts;
  const energy = history === undefined ? null : history.data;
  const estimatedKwh = energy === null ? null : energy.estimated_energy_kwh;
  return <article className={`device-card plug-card ${actual != null && actual.on ? 'is-on' : ''}`} aria-label={name}>
    <div className="card-heading"><span className={`device-icon ${actual != null && actual.on ? 'lit' : ''}`}><Icon kind="plugs" /></span><div><h2>{name}</h2><p>Smart plug</p></div><Badge tone={actual != null && actual.on ? 'warm' : 'neutral'}>{actual == null ? 'Unknown' : actual.on ? 'On' : 'Off'}</Badge></div>
    <div className="power-summary"><div className="power-reading"><strong>{power == null ? '—' : power.toFixed(1)}</strong><span>W<span>Reported power</span></span></div><div className="energy-reading"><strong>{estimatedKwh === null ? '—' : estimatedKwh.toFixed(2)}</strong><span>kWh<span>Estimated, last 24h</span><span>{energy === null ? 'Awaiting history' : estimatedKwh === null ? 'Insufficient data' : `${duration(energy.energy_observed_ms)} covered`}</span></span></div></div>
    <Freshness actual={value.power_actual} receivedAt={plug.receivedAt} live={live} />
    <StatePair requested={value.target_value == null ? '—' : label(value.target_value)} reported={actual == null ? null : actual.on ? 'On' : 'Off'} target={value.target} />
    <div className="plug-controls">{[true, false].map(on => <button key={String(on)} className={`button ${on ? 'primary' : 'off-button'}`} disabled={!live || busy} aria-label={`Turn ${on ? 'on' : 'off'} ${name}`} onClick={() => client.command(`plug:${value.device}`, { kind: 'SetPlugPower', device: value.device, on })}><span aria-hidden="true">⏻</span> Turn {on ? 'on' : 'off'}</button>)}</div>
    <Feedback status={status} /><Freshness actual={value.actual} receivedAt={plug.receivedAt} live={live} />
    <div className="history-heading"><span className="section-kicker">POWER · LAST 24 HOURS</span>{history !== undefined && history.loading && <small>Updating…</small>}</div>
    {history !== undefined && history.data !== null && <PowerHistoryChart history={history.data} device={name} />}
    {history === undefined || (history.loading && history.data === null) ? <div className="history-placeholder">{live ? 'Loading power history…' : 'History is available when connected.'}</div> : null}
    {history !== undefined && history.error !== null && <div className="history-error" role="status"><span>{history.error}</span><button className="text-button" disabled={!live || history.loading} onClick={() => client.loadPlugPowerHistory(value.device)}>Try again</button></div>}
    <details className="device-details"><summary>Automation & device</summary>
      {value.kill_switch_rules.length === 0 && <p>No automatic power-off rules.</p>}
      {value.kill_switch_rules.map(rule => <div className="automation" key={rule.rule_name}><div className="detail-heading"><strong>{label(rule.rule_name)}</strong><Badge tone="neutral">{label(rule.state)}</Badge></div><p>Turns off below {rule.threshold_watts} W for {duration(rule.holdoff_secs * 1000)}.</p>
        {rule.idle_since_ago_ms != null && <p>Idle for {duration(rule.idle_since_ago_ms + Date.now() - plug.receivedAt)}</p>}
      </div>)}
      <small className="device-id">{value.device}</small>
    </details>
  </article>;
}

function HeatingCard({ zone, live, histories, client }: { zone: Timed<HeatingZone>; live: boolean; histories: ReadonlyMap<string, HistoryStatus>; client: DashboardClient }) {
  const value = zone.value;
  const timer = value.min_cycle_remaining_secs > 0 ? { title: 'Minimum run', seconds: value.min_cycle_remaining_secs }
    : value.min_pause_remaining_secs > 0 ? { title: 'Minimum pause', seconds: value.min_pause_remaining_secs } : null;
  return <section className="heating-zone" id={sectionId(value.name)} aria-label={label(value.name)}>
    <div className="zone-header"><div className="zone-title"><span className="device-icon"><Icon kind="heating" /></span><div><h2>{label(value.name)}</h2><p>{label(value.relay_device)} relay · {value.trvs.length} {value.trvs.length === 1 ? 'valve' : 'valves'}</p></div></div>
      <div className="relay-state"><span><small>REQUESTED</small><strong>{value.target_value == null ? 'Unknown' : value.target_value === 'heating' ? 'Heating' : 'Off'}</strong></span><span><small>REPORTED RELAY</small><strong>{!value.relay_state_known ? 'Unknown' : value.relay_on ? 'On' : 'Off'}</strong></span><span><small>THERMOSTAT</small><strong>{temperature(value.relay_temperature)}</strong></span></div>
      {timer !== null && <Badge tone="warm">{timer.title} · {Math.max(0, Math.ceil(timer.seconds - (Date.now() - zone.receivedAt) / 1000))}s</Badge>}
    </div>
    <Freshness actual={value.actual} receivedAt={zone.receivedAt} live={live} />
    <div className="valve-grid">{value.trvs.map(valve => <ValveCard key={valve.device} valve={valve} receivedAt={zone.receivedAt} live={live} history={histories.get(valve.device)} retry={() => client.loadHistory(valve.device)} />)}</div>
  </section>;
}

function ValveCard({ valve, receivedAt, live, history, retry }: { valve: Valve; receivedAt: number; live: boolean; history: HistoryStatus | undefined; retry: () => void }) {
  return <article className="valve-card" aria-label={label(valve.device)}>
    <div className="valve-heading"><h3 title={valve.device}>{label(valve.device)}</h3><Badge tone={valve.battery !== null && valve.battery < 20 ? 'danger' : 'neutral'}>{valve.battery === null ? 'Battery unknown' : `Battery ${valve.battery}%`}</Badge></div>
    <div className="valve-readings"><div className="valve-temperature"><span className="field-label">TEMPERATURE</span><strong>{temperature(valve.local_temperature)}<small>C</small></strong></div><div><span className="field-label">REQUESTED</span><strong>{valveTarget(valve.target_value)}</strong><small>{valve.target == null ? 'No target' : `${label(valve.target.owner)} · ${label(valve.target.phase)}`}</small></div><div><span className="field-label">REPORTED SETPOINT</span><strong>{temperature(valve.setpoint)}C</strong><small>{valve.pi_heating_demand === null ? label(valve.running_state) : `${valve.pi_heating_demand}% heating demand`}</small></div></div>
    <Freshness actual={valve.actual} receivedAt={receivedAt} live={live} />
    {(valve.inhibited || valve.forced) && <p className="valve-notice">{valve.inhibited ? 'Open-window hold is active.' : valve.target_value != null && valve.target_value.kind === 'forced_open' ? `Valve held open: ${label(valve.target_value.reason)}.` : 'Valve held open by the controller.'}</p>}
    <div className="history-heading"><span className="section-kicker">LAST 24 HOURS</span>{history !== undefined && history.loading && <small>Updating…</small>}</div>
    {history !== undefined && history.data !== null && <HistoryChart history={history.data} device={label(valve.device)} />}
    {history === undefined || (history.loading && history.data === null) ? <div className="history-placeholder">{live ? 'Loading valve history…' : 'History is available when connected.'}</div> : null}
    {history !== undefined && history.error !== null && <div className="history-error" role="status"><span>{history.error}</span><button className="text-button" disabled={!live || history.loading} onClick={retry}>Try again</button></div>}
    <details className="schedule-details"><summary>Schedule <span>{valve.schedule === '' ? 'None' : label(valve.schedule)}</span></summary><p>{valve.schedule_summary || 'No schedule configured.'}</p><small className="device-id">{valve.device}</small></details>
  </article>;
}
