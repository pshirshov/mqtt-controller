import { expect, test } from '@playwright/test';
import { commandSchema, serverSchema } from '../src/protocol';

// Specified: starting Boost replaces the duration/start controls with target/cancel.
test('valve boost starts at 22 degrees, edits its target and can be cancelled', async ({ page }) => {
  await page.goto('/#heating');
  const valve = page.getByRole('article', { name: 'Ensuite', exact: true });
  const duration = valve.getByRole('combobox', { name: 'Boost duration for Ensuite' });
  await expect(duration.locator('option')).toHaveText(['30m', '1h', '1h 30m', '2h', '3h', '4h', '6h']);
  await duration.selectOption('90');
  await valve.getByRole('button', { name: 'Boost Ensuite', exact: true }).click();
  const target = valve.getByRole('spinbutton', { name: 'Boost target for Ensuite' });
  await expect(target).toHaveValue('22');
  await expect(duration).toHaveCount(0);
  await expect(valve.getByRole('button', { name: 'Boost Ensuite', exact: true })).toHaveCount(0);
  await expect(valve.getByText(/Boost.*1h 30m remaining/)).toBeVisible();
  await target.fill('23.5');
  await target.press('Enter');
  await expect(target).toHaveValue('23.5');
  await valve.getByRole('button', { name: 'Cancel boost for Ensuite' }).click();
  await expect(target).toHaveCount(0);
  await expect(valve.getByRole('button', { name: 'Boost Ensuite', exact: true })).toBeEnabled();
});

for (const width of [1280, 390]) {
  test(`cancel does not submit a dirty target at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    const sent: string[] = [];
    await page.routeWebSocket('**/ws', socket => {
      const server = socket.connectToServer();
      socket.onMessage(data => {
        const message = JSON.parse(data.toString());
        if (message.type === 'Command') sent.push(commandSchema.parse(message.command).kind);
        server.send(data);
      });
    });
    await page.goto('/#heating');
    const valve = page.getByRole('article', { name: 'Ensuite', exact: true });
    await valve.getByRole('button', { name: 'Boost Ensuite', exact: true }).click();
    const target = valve.getByRole('spinbutton', { name: 'Boost target for Ensuite' });
    await target.fill('23.5');
    await valve.getByRole('button', { name: 'Cancel boost for Ensuite' }).click();
    await expect(target).toHaveCount(0);
    expect(sent).toEqual(['StartValveBoost', 'CancelValveBoost']);
    await valve.getByRole('button', { name: 'Boost Ensuite', exact: true }).click();
    await target.fill('');
    await valve.getByRole('button', { name: 'Cancel boost for Ensuite' }).click();
    await expect(target).toHaveCount(0);
    expect(sent).toEqual(['StartValveBoost', 'CancelValveBoost', 'StartValveBoost', 'CancelValveBoost']);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await valve.getByRole('button', { name: 'Boost Ensuite', exact: true }).click();
    await expect(target).toHaveValue('22');
    await page.screenshot({ path: `test-results/boost-${width}.png`, fullPage: true });
  });
}

test('boost target validates input, reports save failure, and keeps the original deadline', async ({ page }) => {
  const deadlines: number[] = [];
  const targetCommands: number[] = [];
  await page.routeWebSocket('**/ws', socket => {
    const server = socket.connectToServer();
    socket.onMessage(data => {
      const message = JSON.parse(data.toString());
      if (message.type === 'Command') {
        const command = commandSchema.parse(message.command);
        if (command.kind === 'SetValveBoostTarget') {
          targetCommands.push(command.temperature);
          if (command.temperature === 24) {
            socket.send(JSON.stringify({ type: 'CommandResult', request_id: message.request_id, error: 'Could not save boost: test failure' }));
            return;
          }
        }
      }
      server.send(data);
    });
    server.onMessage(data => {
      const message = serverSchema.parse(JSON.parse(data.toString()));
      if (message.type === 'Entity' && message.kind === 'HeatingZone') {
        for (const valve of message.data.trvs) if (valve.boost !== null) deadlines.push(valve.boost.ends_at_epoch_ms);
      }
      socket.send(data);
    });
  });
  await page.goto('/#heating');
  const valve = page.getByRole('article', { name: 'Ensuite', exact: true });
  await valve.getByRole('button', { name: 'Boost Ensuite', exact: true }).click();
  const target = valve.getByRole('spinbutton', { name: 'Boost target for Ensuite' });
  for (const invalid of ['31', '22.25', '']) {
    await target.fill(invalid);
    await target.press('Enter');
    expect(await target.evaluate(input => (input as HTMLInputElement).validity.valid)).toBe(false);
  }
  expect(targetCommands).toEqual([]);
  await target.fill('23.5');
  await target.press('Enter');
  await expect(target).toHaveValue('23.5');
  await expect.poll(() => deadlines.length).toBe(2);
  expect(deadlines[1]).toBe(deadlines[0]);
  await target.fill('24');
  await target.press('Enter');
  await expect(valve.getByText('Could not save boost: test failure')).toBeVisible();
  await expect(target).toHaveValue('23.5');
  await expect(valve.getByRole('button', { name: 'Cancel boost for Ensuite' })).toBeEnabled();
});

test('controller expiry removes boost controls and restores the suppression notice', async ({ page }) => {
  let expire: (() => void) | null = null;
  await page.routeWebSocket('**/ws', socket => {
    const server = socket.connectToServer();
    server.onMessage(data => {
      const message = serverSchema.parse(JSON.parse(data.toString()));
      if (message.type === 'StateSnapshot') {
        const zone = message.heating_zones[0]!;
        const valve = zone.trvs.find(valve => valve.device === 'sonoff-trv-ensuite')!;
        valve.heat_demand_enabled = false;
        valve.boost = { temperature: 22, ends_at_epoch_ms: Date.now(), remaining_ms: 0 };
        expire = () => {
          valve.boost = null;
          socket.send(JSON.stringify({ type: 'Entity', kind: 'HeatingZone', data: zone }));
        };
        socket.send(JSON.stringify(message));
      } else socket.send(data);
    });
  });
  await page.goto('/#heating');
  const valve = page.getByRole('article', { name: 'Ensuite', exact: true });
  await expect(valve.getByText('Boost · Awaiting controller expiry')).toBeVisible();
  await expect(valve.getByText(/Boost temporarily enables demand/)).toBeVisible();
  await expect(valve.getByRole('switch')).not.toBeChecked();
  await expect(valve.getByRole('button', { name: 'Cancel boost for Ensuite' })).toBeEnabled();
  if (expire === null) throw new Error('No controller snapshot');
  (expire as () => void)();
  await expect(valve.getByRole('spinbutton')).toHaveCount(0);
  await expect(valve.getByText(/Demand from this valve is ignored/)).toBeVisible();
});
