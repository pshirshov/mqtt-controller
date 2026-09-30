import { duration, label } from './format';
import type { ControlCommand, KillSwitchRule, TimedAction } from './protocol';
import { validTimeExpression } from './timeExpression';

/** "turn_off → ensuite" as deployed, rendered for people. */
export function describeAction(action: string): string {
  const [effect, target] = action.split(' → ');
  const words = (effect ?? action).replaceAll('_', ' ');
  return target === undefined ? words : `${words} → ${label(target)}`;
}

export type TimedActionDrafts = Record<string, string>;

export function timedActionDrafts(actions: TimedAction[]): TimedActionDrafts {
  return Object.fromEntries(actions.map(action => [action.binding, action.time]));
}

export function timedActionProblem(actions: TimedAction[], drafts: TimedActionDrafts): string | null {
  const invalid = actions.find(action => !validTimeExpression((drafts[action.binding] ?? '').trim()));
  return invalid === undefined ? null
    : `${label(invalid.binding)}: times are HH:MM, sunrise or sunset with an optional ±HH:MM offset, or min(a, b) / max(a, b).`;
}

/** Commands that bring the controller to the drafted times; empty when nothing changed. */
export function timedActionCommands(actions: TimedAction[], drafts: TimedActionDrafts): [string, ControlCommand][] {
  return actions
    .filter(action => (drafts[action.binding] ?? '').trim() !== action.time)
    .map(action => [action.binding, { kind: 'SetTimedActionTime', binding: action.binding, time: (drafts[action.binding] ?? '').trim() }]);
}

export function TimedActionRows({ actions, drafts, onChange }: { actions: TimedAction[]; drafts: TimedActionDrafts; onChange: (drafts: TimedActionDrafts) => void }) {
  return <div className="dialog-rows">{actions.map(action => {
    const name = label(action.binding);
    const value = drafts[action.binding] ?? '';
    return <div className="dialog-row" key={action.binding}>
      <span><strong>{name}</strong><small>{describeAction(action.action)}{action.overridden ? ' · Override active' : ''}</small></span>
      <input type="text" list="time-expressions" aria-label={`Time for ${name}`} aria-invalid={!validTimeExpression(value.trim())}
        value={value} onChange={event => onChange({ ...drafts, [action.binding]: event.target.value })} />
    </div>;
  })}</div>;
}

export interface KillSwitchDraft { watts: string; seconds: string }
export type KillSwitchDrafts = Record<string, KillSwitchDraft>;

export function killSwitchDrafts(rules: KillSwitchRule[]): KillSwitchDrafts {
  return Object.fromEntries(rules.map(rule => [rule.rule_name, { watts: String(rule.threshold_watts), seconds: String(rule.holdoff_secs) }]));
}

function parsed(draft: KillSwitchDraft | undefined): { watts: number; seconds: number } | null {
  if (draft === undefined || draft.watts.trim() === '' || draft.seconds.trim() === '') return null;
  const watts = Number(draft.watts);
  const seconds = Number(draft.seconds);
  return Number.isFinite(watts) && watts > 0 && Number.isInteger(seconds) && seconds > 0 ? { watts, seconds } : null;
}

export function killSwitchProblem(rules: KillSwitchRule[], drafts: KillSwitchDrafts): string | null {
  const invalid = rules.find(rule => parsed(drafts[rule.rule_name]) === null);
  return invalid === undefined ? null : `${label(invalid.rule_name)} needs a positive watt threshold and a whole number of seconds.`;
}

export function killSwitchCommands(rules: KillSwitchRule[], drafts: KillSwitchDrafts): [string, ControlCommand][] {
  const commands: [string, ControlCommand][] = [];
  for (const rule of rules) {
    const value = parsed(drafts[rule.rule_name]);
    if (value === null || (value.watts === rule.threshold_watts && value.seconds === rule.holdoff_secs)) continue;
    commands.push([rule.rule_name, { kind: 'SetKillSwitch', binding: rule.rule_name, threshold_watts: value.watts, holdoff_secs: value.seconds }]);
  }
  return commands;
}

export function KillSwitchRows({ rules, drafts, onChange }: { rules: KillSwitchRule[]; drafts: KillSwitchDrafts; onChange: (drafts: KillSwitchDrafts) => void }) {
  return <div className="dialog-rows">{rules.map(rule => {
    const name = label(rule.rule_name);
    const draft = drafts[rule.rule_name] ?? { watts: '', seconds: '' };
    const value = parsed(draft);
    const update = (patch: Partial<KillSwitchDraft>) => onChange({ ...drafts, [rule.rule_name]: { ...draft, ...patch } });
    return <div className="dialog-row kill-switch-row" key={rule.rule_name}>
      <span><strong>{name}</strong><small>Turns off when power stays below the threshold{value === null ? '' : ` for ${duration(value.seconds * 1000)}`}{rule.overridden ? ' · Override active' : ''}</small></span>
      <span className="kill-switch-fields">
        <label className="unit-field">Below <input type="number" aria-label={`Threshold for ${name}`} min="0" step="any" value={draft.watts} onChange={event => update({ watts: event.target.value })} /> W</label>
        <label className="unit-field">for <input type="number" aria-label={`Holdoff for ${name}`} min="1" step="1" value={draft.seconds} onChange={event => update({ seconds: event.target.value })} /> s</label>
      </span>
    </div>;
  })}</div>;
}
