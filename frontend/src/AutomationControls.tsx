import { useState } from 'react';
import { duration, label } from './format';
import type { ControlCommand, KillSwitchRule, TimedAction } from './protocol';
import { validTimeExpression } from './timeExpression';

function TimedActionRow({ action, disabled, command }: { action: TimedAction; disabled: boolean; command: (command: ControlCommand) => void }) {
  const [time, setTime] = useState(action.time);
  const name = label(action.binding);
  const valid = validTimeExpression(time);
  return <form className="override-row" aria-label={`Timed action ${name}`} onSubmit={event => {
    event.preventDefault();
    if (valid) command({ kind: 'SetTimedActionTime', binding: action.binding, time });
  }}>
    <span><strong>{name}</strong><small>{action.action}{action.overridden ? ' · Override' : ''}</small></span>
    <input type="text" aria-label={`Time for ${name}`} aria-invalid={!valid} value={time} onChange={event => setTime(event.target.value)} />
    <button type="submit" className="text-button" disabled={disabled || !valid || time === action.time}>Save</button>
    {action.overridden && <button type="button" className="text-button" disabled={disabled}
      onClick={() => command({ kind: 'ResetTimedActionTime', binding: action.binding })}>Restore default</button>}
  </form>;
}

export function TimedActions({ actions, disabled, command }: { actions: TimedAction[]; disabled: boolean; command: (command: ControlCommand) => void }) {
  if (actions.length === 0) return null;
  return <div className="automation"><div className="detail-heading"><strong>Timed actions</strong></div>
    {actions.map(action => <TimedActionRow key={`${action.binding}:${action.time}`} action={action} disabled={disabled} command={command} />)}
  </div>;
}

/** Deployed state details are rendered by the caller; this edits threshold and holdoff. */
export function KillSwitchEditor({ rule, disabled, command }: { rule: KillSwitchRule; disabled: boolean; command: (command: ControlCommand) => void }) {
  const [watts, setWatts] = useState(String(rule.threshold_watts));
  const [seconds, setSeconds] = useState(String(rule.holdoff_secs));
  const name = label(rule.rule_name);
  const thresholdWatts = Number(watts);
  const holdoffSecs = Number(seconds);
  const valid = watts.trim() !== '' && Number.isFinite(thresholdWatts) && thresholdWatts > 0 && Number.isInteger(holdoffSecs) && holdoffSecs > 0;
  const changed = thresholdWatts !== rule.threshold_watts || holdoffSecs !== rule.holdoff_secs;
  return <form className="override-row" aria-label={`Kill switch ${name}`} onSubmit={event => {
    event.preventDefault();
    if (valid) command({ kind: 'SetKillSwitch', binding: rule.rule_name, threshold_watts: thresholdWatts, holdoff_secs: holdoffSecs });
  }}>
    <label>Below <input type="number" aria-label={`Threshold for ${name}`} min="0" step="any" value={watts} onChange={event => setWatts(event.target.value)} /> W</label>
    <label>for <input type="number" aria-label={`Holdoff for ${name}`} min="1" step="1" value={seconds} onChange={event => setSeconds(event.target.value)} /> s</label>
    <small>{valid ? duration(holdoffSecs * 1000) : 'Positive watts and whole seconds'}</small>
    <button type="submit" className="text-button" disabled={disabled || !valid || !changed}>Save</button>
    {rule.overridden && <button type="button" className="text-button" disabled={disabled}
      onClick={() => command({ kind: 'ResetKillSwitch', binding: rule.rule_name })}>Restore default</button>}
  </form>;
}
