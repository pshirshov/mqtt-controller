import { HistoryChart, PowerHistoryChart, RelayHistoryChart } from './HistoryChart';
import { duration, label } from './format';
import { heatPumpHistory, relayHistory, relayTotals, totalPlugEnergyKwh } from './energy';
import type { HeatingZone, Plug } from './protocol';
import { DashboardClient, type DashboardState, type HistoryStatus, type Timed } from './store';

function HistoryContents<T>({ status, live, retry, children }: {
  status: HistoryStatus<T> | undefined; live: boolean; retry: () => void; children: (data: T) => React.ReactNode;
}) {
  return <>{status !== undefined && status.data !== null && children(status.data)}
    {(status === undefined || status.data === null) && <p className="chart-empty">{live ? 'Awaiting history…' : 'History is available when connected.'}</p>}
    {status !== undefined && status.error !== null && <div className="history-error" role="status">{status.error}<button className="text-button" disabled={!live || status.loading} onClick={retry}>Try again</button></div>}
  </>;
}

const runtime = (value: number | null): string => value === null ? '—' : `${(value / 3600_000).toFixed(2)} h`;
const consumption = (value: number | null): string => value === null ? '—' : `${value.toFixed(2)} kWh`;

export function EnergyPlugs({ state, plugs, client }: { state: DashboardState; plugs: Timed<Plug>[]; client: DashboardClient }) {
  const rooms = [...new Set(plugs.map(plug => plug.value.room ?? 'unassigned'))];
  return <div className="energy-sections">{rooms.map(room => <section className="energy-section" id={`section-${encodeURIComponent(room)}`} aria-label={label(room)} key={room}>
    <div className="room-group-heading"><h2>{label(room)}</h2><span>· {consumption(totalPlugEnergyKwh(state.plugs.filter(plug => (plug.value.room ?? 'unassigned') === room), state.plugHistories))} last 24h</span></div>
    <div className="energy-chart-list">{plugs.filter(plug => (plug.value.room ?? 'unassigned') === room).map(plug => {
      const value = plug.value;
      const name = value.display_name ?? label(value.device);
      const status = state.plugHistories.get(value.device);
      const data = status === undefined ? null : status.data;
      return <article className="energy-chart-row" key={value.device} aria-label={name}>
        <div className="energy-chart-heading"><h3>{name}</h3><span>{consumption(data === null ? null : data.estimated_energy_kwh)} estimated</span></div>
        <p className="energy-coverage">{value.exclude_from_totals ? 'Excluded from totals · ' : ''}{data === null || data.estimated_energy_kwh === null ? 'Insufficient data' : `${duration(data.energy_observed_ms)} covered`}</p>
        <HistoryContents status={status} live={state.ready} retry={() => client.loadPlugPowerHistory(value.device)}>
          {history => <PowerHistoryChart history={history} device={name} compact={true} />}
        </HistoryContents>
      </article>;
    })}</div>
  </section>)}</div>;
}

export function EnergyHeating({ state, zones, query, client }: { state: DashboardState; zones: Timed<HeatingZone>[]; query: string; client: DashboardClient }) {
  const history = state.heatingEnergy.data;
  const relayHistories = history === null ? [] : state.heating.map(zone => relayHistory(history, zone.value.name, zone.value.relay_device));
  const totals = relayTotals(relayHistories);
  const pump = history === null ? null : heatPumpHistory(history);
  const showPump = query === '' || 'heat pump'.includes(query) || (pump !== null && pump.device.includes(query));
  return <div className="energy-sections">
    <div className="energy-overview">
      <div><span>Zone relay-hours · last 24h</span><strong>{runtime(totals.summed_ms)}</strong><small>Sum across zones; simultaneous relays count separately</small></div>
      <div><span>Any relay ON · last 24h</span><strong>{runtime(totals.any_on_ms)}</strong><small>Elapsed time with at least one reported relay ON</small></div>
      <div><span>Heat pump · last 24h</span><strong>{consumption(pump === null ? null : pump.consumed_energy_kwh)}</strong><small>Measured electricity from the energy counter</small></div>
    </div>
    <p className="energy-coverage">Recorded intervals only. Unknown, stale and missing relay intervals are excluded.</p>
    {state.heatingEnergy.error !== null && <div className="history-error" role="status">{state.heatingEnergy.error}<button className="text-button" disabled={!state.ready || state.heatingEnergy.loading} onClick={() => client.loadHeatingEnergy()}>Try again</button></div>}
    {showPump && <section className="energy-section" aria-label="Heat pump" id="section-heat-pump"><div className="room-group-heading"><h2>Heat pump</h2></div>
      <article className="energy-chart-row" aria-label="Heat pump power"><div className="energy-chart-heading"><h3>Electrical power</h3><span>{consumption(pump === null ? null : pump.consumed_energy_kwh)} consumed</span></div>
        {pump === null ? <p className="chart-empty">No heat-pump meter history yet.</p> : <>
          <p className="energy-coverage">{pump.consumed_energy_kwh === null ? 'Insufficient counter readings' : `${duration(pump.observed_ms)} covered`}{pump.resets > 0 && ` · ${pump.resets} counter reset(s); reset intervals excluded`}</p>
          <PowerHistoryChart history={pump} device="Heat pump" compact={true} />
        </>}
      </article>
    </section>}
    {zones.map(zone => {
      const relay = history === null ? null : relayHistory(history, zone.value.name, zone.value.relay_device);
      return <section className="energy-section" key={zone.value.name} aria-label={label(zone.value.name)} id={`section-${encodeURIComponent(zone.value.name)}`}>
        <div className="room-group-heading"><h2>{label(zone.value.name)}</h2><span>· {runtime(relay === null ? null : relay.on_ms)} relay ON last 24h</span></div>
        <div className="energy-chart-list"><article className="energy-chart-row" aria-label={`${label(zone.value.name)} relay`}>
          <div className="energy-chart-heading"><h3>{label(zone.value.relay_device)} · relay</h3><span>{runtime(relay === null ? null : relay.on_ms)} ON</span></div>
          <p className="energy-coverage">{relay === null || relay.observed_ms === 0 ? 'Insufficient data' : `${duration(relay.observed_ms)} covered`}</p>
          {relay !== null && <RelayHistoryChart history={relay} device={label(zone.value.name)} />}
        </article>
          {zone.value.trvs.map(valve => <article className="energy-chart-row" key={valve.device} aria-label={label(valve.device)}>
            <div className="energy-chart-heading"><h3>{label(valve.device)}</h3>{!valve.heat_demand_enabled && <span>Heat demand suppressed</span>}</div>
            <HistoryContents status={state.histories.get(valve.device)} live={state.ready} retry={() => client.loadHistory(valve.device)}>
              {history => <HistoryChart history={history} device={label(valve.device)} compact={true} />}
            </HistoryContents>
          </article>)}
        </div>
      </section>;
    })}
  </div>;
}
