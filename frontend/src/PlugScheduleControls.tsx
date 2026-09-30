import { useRef, useState } from 'react';
import {
  KillSwitchRows, TimedActionRows, killSwitchCommands, killSwitchDrafts, killSwitchProblem, timedActionCommands, timedActionDrafts,
  timedActionProblem, type KillSwitchDrafts, type TimedActionDrafts,
} from './AutomationRows';
import type { Send } from './LightScheduleControls';
import { DialogSection, ScheduleButton, ScheduleDialog } from './ScheduleDialog';
import type { Plug } from './protocol';

interface PlugDraft { times: TimedActionDrafts; rules: KillSwitchDrafts }

/** Timed on/off actions and power-off rules of one plug in a single editor. */
export function PlugScheduleControls({ plug, name, commandKey, disabled, send }: { plug: Plug; name: string; commandKey: string; disabled: boolean; send: Send }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [draft, setDraft] = useState<PlugDraft | null>(null);
  const overridden = plug.timed_actions.some(action => action.overridden) || plug.kill_switch_rules.some(rule => rule.overridden);
  const commands = draft === null ? [] : [
    ...timedActionCommands(plug.timed_actions, draft.times).map(([binding, command]) => [`${commandKey}/timed/${binding}`, command] as const),
    ...killSwitchCommands(plug.kill_switch_rules, draft.rules).map(([binding, command]) => [`${commandKey}/kill/${binding}`, command] as const),
  ];
  const problem = draft === null ? null : timedActionProblem(plug.timed_actions, draft.times) ?? killSwitchProblem(plug.kill_switch_rules, draft.rules);
  return <div className="schedule-controls">
    <ScheduleButton label="Edit schedule" overridden={overridden} disabled={disabled || (plug.timed_actions.length === 0 && plug.kill_switch_rules.length === 0)}
      onClick={() => { setDraft({ times: timedActionDrafts(plug.timed_actions), rules: killSwitchDrafts(plug.kill_switch_rules) }); dialog.current?.showModal(); }} />
    <ScheduleDialog dialogRef={dialog} label={`Schedule for ${name}`} title={`${name} schedule`}
      subtitle="Timed actions and power-off rules are saved on the controller and survive restarts." error={problem}
      canSave={problem === null && commands.length > 0} onClose={() => setDraft(null)}
      onSave={() => { for (const [key, command] of commands) send(key, command); dialog.current?.close(); }}
      onRestore={!overridden ? null : () => {
        for (const action of plug.timed_actions.filter(action => action.overridden)) send(`${commandKey}/timed/${action.binding}`, { kind: 'ResetTimedActionTime', binding: action.binding });
        for (const rule of plug.kill_switch_rules.filter(rule => rule.overridden)) send(`${commandKey}/kill/${rule.rule_name}`, { kind: 'ResetKillSwitch', binding: rule.rule_name });
        dialog.current?.close();
      }}>
      {draft !== null && <>
        <datalist id="time-expressions">{['06:00', '07:00', '08:00', '18:00', '22:00', '23:00', 'sunrise', 'sunset', 'sunset-00:30'].map(value => <option key={value} value={value} />)}</datalist>
        {plug.timed_actions.length > 0 && <DialogSection title="Timed actions" hint="Daily actions on this plug. Times are HH:MM or sunrise/sunset with an optional ±HH:MM offset.">
          <TimedActionRows actions={plug.timed_actions} drafts={draft.times} onChange={times => setDraft({ ...draft, times })} />
        </DialogSection>}
        {plug.kill_switch_rules.length > 0 && <DialogSection title="Power-off rules" hint="Running idle timers pick up a new holdoff immediately.">
          <KillSwitchRows rules={plug.kill_switch_rules} drafts={draft.rules} onChange={rules => setDraft({ ...draft, rules })} />
        </DialogSection>}
      </>}
    </ScheduleDialog>
  </div>;
}
