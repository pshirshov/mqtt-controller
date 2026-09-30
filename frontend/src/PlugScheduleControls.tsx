import { useRef, useState } from 'react';
import type { Send } from './LightScheduleControls';
import { DialogSection, ScheduleButton, ScheduleDialog } from './ScheduleDialog';
import { duration } from './format';
import type { Plug, PlugAction, PlugSchedule } from './protocol';
import { validTimeExpression } from './timeExpression';

interface ActionDraft { key: number; time: string; action: PlugAction }
interface KillSwitchDraft { watts: string; seconds: string }
interface PlugDraft { actions: ActionDraft[]; killSwitch: KillSwitchDraft | null }

const ACTIONS: { value: PlugAction; label: string }[] = [{ value: 'on', label: 'Turn on' }, { value: 'off', label: 'Turn off' }, { value: 'toggle', label: 'Toggle' }];
const DEFAULT_KILL_SWITCH: KillSwitchDraft = { watts: '5', seconds: '600' };
let nextKey = 1;

function draftOf(schedule: PlugSchedule): PlugDraft {
  return {
    actions: schedule.timed_actions.map(action => ({ key: nextKey++, time: action.time, action: action.action })),
    killSwitch: schedule.kill_switch == null ? null : { watts: String(schedule.kill_switch.threshold_watts), seconds: String(schedule.kill_switch.holdoff_secs) },
  };
}

function parsedKillSwitch(draft: KillSwitchDraft): { threshold_watts: number; holdoff_secs: number } | null {
  if (draft.watts.trim() === '' || draft.seconds.trim() === '') return null;
  const watts = Number(draft.watts);
  const seconds = Number(draft.seconds);
  return Number.isFinite(watts) && watts > 0 && Number.isInteger(seconds) && seconds > 0 ? { threshold_watts: watts, holdoff_secs: seconds } : null;
}

export function plugProblem(draft: PlugDraft): string | null {
  const invalid = draft.actions.findIndex(action => !validTimeExpression(action.time.trim()));
  if (invalid >= 0) return `Action ${invalid + 1}: times are HH:MM, sunrise or sunset with an optional ±HH:MM offset, or min(a, b) / max(a, b).`;
  if (draft.actions.some(action => action.time.trim() === '24:00')) return 'Timed actions cannot run at 24:00.';
  if (draft.killSwitch !== null && parsedKillSwitch(draft.killSwitch) === null) return 'The power-off rule needs a positive watt threshold and a whole number of seconds.';
  return null;
}

function planOf(draft: PlugDraft): { timed_actions: PlugSchedule['timed_actions']; kill_switch: { threshold_watts: number; holdoff_secs: number } | null } {
  return {
    timed_actions: draft.actions.map(action => ({ time: action.time.trim(), action: action.action })),
    kill_switch: draft.killSwitch === null ? null : parsedKillSwitch(draft.killSwitch),
  };
}

/** Daily on/off actions and the power-off rule of one plug; saved as one schedule. */
export function PlugScheduleControls({ plug, name, commandKey, disabled, send }: { plug: Plug; name: string; commandKey: string; disabled: boolean; send: Send }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [draft, setDraft] = useState<PlugDraft | null>(null);
  const problem = draft === null ? null : plugProblem(draft);
  const plan = draft === null ? null : planOf(draft);
  const changed = plan !== null && (JSON.stringify(plan.timed_actions) !== JSON.stringify(plug.schedule.timed_actions)
    || JSON.stringify(plan.kill_switch) !== JSON.stringify(plug.schedule.kill_switch ?? null));
  const update = (patch: Partial<PlugDraft>) => { if (draft !== null) setDraft({ ...draft, ...patch }); };
  const updateAction = (index: number, patch: Partial<ActionDraft>) => { if (draft !== null) update({ actions: draft.actions.map((action, n) => n === index ? { ...action, ...patch } : action) }); };
  const killSwitch = draft === null ? null : draft.killSwitch;
  const holdoff = killSwitch === null ? null : parsedKillSwitch(killSwitch);
  return <div className="schedule-controls">
    <ScheduleButton label="Edit schedule" overridden={plug.schedule.overridden} disabled={disabled}
      onClick={() => { setDraft(draftOf(plug.schedule)); dialog.current?.showModal(); }} />
    <ScheduleDialog dialogRef={dialog} label={`Schedule for ${name}`} title={`${name} schedule`}
      subtitle="Daily actions and the power-off rule are saved on the controller and survive restarts." error={problem}
      canSave={changed && problem === null} onClose={() => setDraft(null)}
      onSave={() => { if (plan !== null) send(commandKey, { kind: 'SetPlugSchedule', device: plug.device, ...plan }); dialog.current?.close(); }}
      onRestore={!plug.schedule.overridden ? null : () => { send(commandKey, { kind: 'ResetPlugSchedule', device: plug.device }); dialog.current?.close(); }}>
      {draft !== null && <>
        <datalist id="time-expressions">{['06:00', '07:00', '08:00', '18:00', '22:00', '23:00', 'sunrise', 'sunset', 'sunset-00:30'].map(value => <option key={value} value={value} />)}</datalist>
        <DialogSection title="Timed actions" hint="Times are HH:MM or sunrise/sunset with an optional ±HH:MM offset. Each action fires once a day.">
          <div className="dialog-rows">
            {draft.actions.map((action, index) => <div className="dialog-row action-row" key={action.key}>
              <select aria-label={`Action ${index + 1}`} value={action.action} onChange={event => updateAction(index, { action: event.target.value as PlugAction })}>
                {ACTIONS.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
              </select>
              <label className="unit-field">at <input type="text" list="time-expressions" aria-label={`Time of action ${index + 1}`} aria-invalid={!validTimeExpression(action.time.trim())}
                value={action.time} onChange={event => updateAction(index, { time: event.target.value })} /></label>
              <button type="button" className="text-button" aria-label={`Remove action ${index + 1}`} onClick={() => update({ actions: draft.actions.filter((_, n) => n !== index) })}>Remove</button>
            </div>)}
            {draft.actions.length === 0 && <p className="schedule-hint">No timed actions.</p>}
            <button type="button" className="text-button add-row" onClick={() => update({ actions: [...draft.actions, { key: nextKey++, time: '', action: draft.actions.length % 2 === 0 ? 'on' : 'off' }] })}>Add action</button>
          </div>
        </DialogSection>
        <DialogSection title="Power-off rule" hint={plug.schedule.power_metered ? 'Turns the plug off once its power stays below the threshold for the holdoff. A running idle timer picks up new values immediately.' : 'This plug does not report power, so it cannot have a power-off rule.'}>
          {killSwitch === null
            ? <button type="button" className="text-button add-row" disabled={!plug.schedule.power_metered} onClick={() => update({ killSwitch: DEFAULT_KILL_SWITCH })}>Add power-off rule</button>
            : <div className="dialog-row kill-switch-row">
              <span><strong>Power-off rule</strong><small>{holdoff === null ? 'Positive watts and whole seconds' : `Off after ${duration(holdoff.holdoff_secs * 1000)} below ${holdoff.threshold_watts} W`}</small></span>
              <span className="kill-switch-fields">
                <label className="unit-field">Below <input type="number" aria-label="Power-off threshold" min="0" step="any" value={killSwitch.watts} onChange={event => update({ killSwitch: { ...killSwitch, watts: event.target.value } })} /> W</label>
                <label className="unit-field">for <input type="number" aria-label="Power-off holdoff" min="1" step="1" value={killSwitch.seconds} onChange={event => update({ killSwitch: { ...killSwitch, seconds: event.target.value } })} /> s</label>
                <button type="button" className="text-button" onClick={() => update({ killSwitch: null })}>Remove rule</button>
              </span>
            </div>}
        </DialogSection>
      </>}
    </ScheduleDialog>
  </div>;
}
