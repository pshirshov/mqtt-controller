# Scheduled switch steps

A room can override its switch scene cycle for selected scene-schedule slots:

```nix
switchSteps.night = [
  { sceneId = 3; lights = [ "hue-l-bathroom-wall/11" ]; }
  { sceneId = 3; lights = [ "hue-l-bathroom-wall/11" "hue-l-bathroom-ceiling/11" ]; }
  { sceneId = 2; lights = [ "hue-l-bathroom-wall/11" ]; }
  { sceneId = 2; lights = [ "hue-l-bathroom-wall/11" "hue-l-bathroom-ceiling/11" ]; }
];
```

`sceneId` references a scene defined by the room. Every step sends that scene's
ON state, brightness, color temperature and transition to the listed lights,
and explicitly turns **all other room members OFF**, using the room's off
transition. This uses individual endpoint commands, without adding Zigbee groups
or stored scenes. The JSON names are `switch_steps` and `scene_id`.

`scene_cycle` starts at the first step and advances once per ON press, wrapping
after the last step. An OFF command, observed external OFF, explicit whole-group
scene recall, or entering another slot resets the cycle. Slot changes alone do
not change the lights; the next ON press chooses the new slot's first step or
regular scene. Slots without an override keep ordinary whole-group scene cycles.

`scene_toggle` still toggles off when on. `scene_toggle_cycle` advances within its
configured press window and turns off after the window expires. Brightness
press/hold/release addresses the active step's selected lights without advancing
the cycle, retaining that selection across slot changes until another ON press.
Before a step is selected, brightness controls retain their ordinary group scope.
Dashboard scene buttons remain explicit whole-group controls.

Steps take manual ownership from `on-off` motion automation. Active `off-only`
claims remain effective. Commands never fabricate bulb observations; step
confirmation requires reports matching all selected ON and excluded OFF targets.

Validation rejects unknown slots/scenes, OFF scenes, empty sequences or light
selections, duplicates, nonmember endpoints, and multiple configured endpoints
for a step-controlled device (telemetry is currently tracked per device).

The bathroom configuration uses the same six steps in evening, night and morning:
dim wall → dim both → warm-bright wall → warm-bright both → cool-bright wall →
cool-bright both → repeat. Daytime retains its existing group cycle. Bathroom
motion rules also start with the dim scene outside daytime, targeting only the
wall light.
