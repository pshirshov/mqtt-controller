import { expect, test } from '@playwright/test';

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
});

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
