import { expect, test } from '@playwright/test';
import { plugPowerHistorySchema, snapshotSchema } from '../src/protocol';

for (const screen of [
  { name: 'desktop', width: 1280, height: 720 },
  { name: 'mobile', width: 390, height: 844 },
]) {
  for (const section of [
    { page: 'lights', menu: 'Lights rooms' },
    { page: 'plugs', menu: 'Plugs rooms' },
    { page: 'heating', menu: 'Heating zones' },
  ]) {
    test(`${section.page} shortcuts scroll to visible sections on ${screen.name}`, async ({ page }) => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.setViewportSize({ width: screen.width, height: screen.height });
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.routeWebSocket('**/ws', socket => {
        const server = socket.connectToServer();
        server.onMessage(data => {
          const parsed = snapshotSchema.safeParse(JSON.parse(data.toString()));
          if (!parsed.success) { socket.send(data); return; }
          const snapshot = parsed.data;
          const rooms = ['office', 'office', 'living-room', 'small-bedroom'];
          socket.send(JSON.stringify({
            ...snapshot,
            rooms: rooms.map((room, index) => ({ ...snapshot.rooms[0], name: `zone-${index}`, room })),
            plugs: rooms.map((room, index) => ({ ...snapshot.plugs[0], device: `plug-${index}`, room })),
            heating_zones: [...new Set(rooms)].map((name, index) => ({
              ...snapshot.heating_zones[0], name,
              trvs: snapshot.heating_zones[0]!.trvs.map(valve => ({ ...valve, device: `${valve.device}-${index}` })),
            })),
          }));
        });
      });
      await page.goto(`/#${section.page}`);
      const shortcuts = page.getByRole('group', { name: section.menu });
      await expect(shortcuts.getByRole('button')).toHaveText(['Office', 'Living room', 'Small bedroom']);
      await expect(page.locator('.section-nav')).toHaveCount(1);
      await shortcuts.getByRole('button', { name: 'Small bedroom' }).click();
      const heading = page.getByRole('region', { name: 'Small bedroom', exact: true }).getByRole('heading', { name: 'Small bedroom', exact: true });
      await expect(heading).toBeInViewport();
      expect(await page.evaluate(() => window.scrollY)).toBeGreaterThan(0);
      if (screen.name === 'mobile') {
        const bounds = await heading.boundingBox();
        const sidebar = await page.locator('.sidebar').boundingBox();
        if (bounds === null || sidebar === null) throw new Error('Missing layout bounds');
        expect(bounds.y).toBeGreaterThanOrEqual(sidebar.y + sidebar.height);
      }
      await shortcuts.getByRole('button', { name: 'Office', exact: true }).focus();
      await page.keyboard.press('Enter');
      await expect(page.getByRole('region', { name: 'Office', exact: true }).getByRole('heading', { name: 'Office', exact: true })).toBeInViewport();
      await expect(page).toHaveURL(new RegExp(`#${section.page}$`));
      const search = page.getByRole('searchbox', { name: `Search ${section.page}` });
      await search.fill('small-bedroom');
      await expect(shortcuts.getByRole('button')).toHaveText(['Small bedroom']);
      await search.fill('no-matching-device');
      await expect(shortcuts).toHaveCount(0);
      await search.fill('');
      await expect(shortcuts.getByRole('button')).toHaveCount(3);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
      await page.screenshot({ path: `test-results/${section.page}-shortcuts-${screen.name}.png`, fullPage: true });
      const next = section.page === 'heating' ? 'Lights' : 'Heating';
      await page.getByRole('link', { name: new RegExp(`^${next}`) }).click();
      await expect(shortcuts).toHaveCount(0);
      expect(errors).toEqual([]);
    });
  }
}

test('light OFF and scenes send acknowledged commands and update reported state', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  const card = page.getByRole('article', { name: 'Ensuite', exact: true });
  await expect(page.getByLabel('Controller connection: Connected')).toBeVisible();
  await card.getByRole('button', { name: 'Turn off Ensuite' }).click();
  await expect(card.locator('.state-pair')).toContainText('Off');
  await expect(card.getByRole('status')).toHaveCount(0);
  await card.getByRole('button', { name: 'Recall scene 2 in Ensuite' }).click();
  await expect(card.locator('.state-pair')).toContainText('Scene 2');
  await expect(card.locator('.state-pair')).toContainText('On');
  await expect(card.getByRole('status')).toHaveCount(0);
  await card.getByText('Lights & automation').click();
  await expect(card.getByText('Motion enabled')).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Event Log' })).toHaveCount(0);
  await page.screenshot({ path: 'test-results/lights-desktop.png', fullPage: true });
  expect(errors).toEqual([]);
});

test('plugs expose explicit on and off actions', async ({ page }) => {
  await page.goto('/#plugs');
  const card = page.getByRole('article', { name: '3d printer' });
  await expect(page.getByRole('region', { name: 'Office' })).toContainText('3d printer');
  await expect(card.getByRole('button', { name: 'Turn off 3d printer' })).toBeEnabled();
  await card.getByRole('button', { name: 'Turn off 3d printer' }).click();
  await expect(card.locator('.state-pair')).toContainText('Off');
  await expect(card.getByRole('status')).toHaveCount(0);
  await card.getByRole('button', { name: 'Turn on 3d printer' }).click();
  await expect(card.locator('.state-pair')).toContainText('On');
  await expect(card.getByRole('status')).toHaveCount(0);
  await expect(card.getByRole('img', { name: /power history for the last 24 hours/ })).toBeVisible();
  await expect(card.locator('.energy-reading')).toContainText('1.68');
  await expect(card.locator('.energy-reading')).toContainText('23h 59m covered');
});

// Regression: missing history, measured zero, and partial coverage are distinct.
for (const scenario of [
  { name: 'unknown', kwh: null, observed: 0, reading: '—', coverage: 'Insufficient data' },
  { name: 'measured zero', kwh: 0, observed: 60_000, reading: '0.00', coverage: '1m covered' },
  { name: 'partial coverage', kwh: 0.12, observed: 3600_000, reading: '0.12', coverage: '1h 0m covered' },
]) {
  test(`plug energy displays ${scenario.name}`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.routeWebSocket('**/ws', socket => {
      const server = socket.connectToServer();
      server.onMessage(data => {
        const parsed = plugPowerHistorySchema.safeParse(JSON.parse(data.toString()));
        socket.send(parsed.success ? JSON.stringify({
          ...parsed.data, estimated_energy_kwh: scenario.kwh, energy_observed_ms: scenario.observed,
        }) : data);
      });
    });
    await page.goto('/#plugs');
    const energy = page.getByRole('article', { name: '3d printer' }).locator('.energy-reading');
    await expect(energy.locator('strong')).toHaveText(scenario.reading);
    await expect(energy).toContainText(scenario.coverage);
    await page.setViewportSize({ width: 390, height: 844 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    expect(errors).toEqual([]);
  });
}

test('light zones are grouped by physical room', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('region', { name: 'Ensuite' })).toContainText('Ensuite');
});

test('heating retains target, actual, freshness, zero battery and per-valve history', async ({ page }) => {
  await page.goto('/#heating');
  await expect(page.getByRole('img', { name: /temperature and setpoint history/ })).toHaveCount(2);
  await expect(page.locator('.zone-title')).toContainText('Master bedroom wall relay · 2 valves');
  await expect(page.getByText('Battery 0%')).toBeVisible();
  await expect(page.getByText('Open-window hold is active.')).toBeVisible();
  expect(await page.locator('.chart-actual').first().getAttribute('d')).toContain('L');
  await page.screenshot({ path: 'test-results/heating-desktop.png', fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: 'test-results/heating-mobile.png', fullPage: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});

test('clicking a chart opens recorded values at the selected timestamp', async ({ page }) => {
  await page.goto('/#heating');
  const chart = page.getByRole('img', { name: /temperature and setpoint history/ }).first();
  await expect(chart).toBeVisible();
  const bounds = await chart.boundingBox();
  if (bounds === null) throw new Error('chart has no layout box');
  await chart.click({ position: { x: bounds.width * 0.4, y: bounds.height * 0.5 } });
  const table = chart.locator('xpath=..').locator('.history-data');
  await expect(table).toHaveAttribute('open', '');
  const selected = table.locator('tr[aria-current="time"]');
  await expect(selected).toHaveCount(1);
  expect(await selected.evaluate(row => {
    const wrapper = row.closest('.history-table-wrap');
    if (wrapper === null) return false;
    const rowBounds = row.getBoundingClientRect();
    const wrapperBounds = wrapper.getBoundingClientRect();
    return rowBounds.top >= wrapperBounds.top && rowBounds.bottom <= wrapperBounds.bottom;
  })).toBe(true);
});

test('connection interruption pauses controls and recovers with a new snapshot', async ({ page, context }) => {
  await page.goto('/');
  const off = page.getByRole('button', { name: 'Turn off Ensuite' });
  await expect(off).toBeEnabled();
  await context.setOffline(true);
  await expect(off).toBeDisabled({ timeout: 20_000 });
  await context.setOffline(false);
  await expect(off).toBeEnabled({ timeout: 20_000 });
  await off.click();
  await expect(page.getByRole('article', { name: 'Ensuite', exact: true }).locator('.state-pair')).toContainText('Off');
  await expect(page.getByRole('status')).toHaveCount(0);
});

test('follows the operating system dark color scheme', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto('/');
  const light = await page.locator(':root').evaluate(element => getComputedStyle(element).backgroundColor);
  await page.emulateMedia({ colorScheme: 'dark' });
  const dark = await page.locator(':root').evaluate(element => getComputedStyle(element).backgroundColor);
  expect(light).not.toBe(dark);
  expect(dark).toBe('rgb(16, 22, 20)');
});
