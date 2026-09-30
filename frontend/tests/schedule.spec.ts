import { expect, test } from '@playwright/test';

test('valve schedule editor saves a weekly override and restores it from the dialog', async ({ page }) => {
  await page.goto('/#heating');
  const valve = page.getByRole('article', { name: 'Ensuite', exact: true });
  await valve.getByRole('button', { name: 'Edit schedule' }).click();
  const editor = page.getByRole('dialog', { name: 'Schedule for Ensuite' });
  await expect(editor).toBeVisible();
  await expect(editor.getByRole('tab', { name: 'Monday' })).toHaveAttribute('aria-selected', 'true');
  await expect(editor.getByRole('button', { name: 'Save schedule' })).toBeDisabled();
  await editor.getByRole('spinbutton', { name: 'Temperature for period 2' }).fill('20.3');
  await editor.getByRole('button', { name: 'all days' }).click();
  await editor.getByRole('button', { name: 'Save schedule' }).click();
  await expect(editor).not.toBeVisible();
  await expect(valve.getByRole('button', { name: 'Edit schedule · Override active' })).toBeVisible();
  await expect(valve.getByRole('status')).toHaveCount(0);

  await valve.getByRole('button', { name: /Override active/ }).click();
  await editor.getByRole('tab', { name: 'Friday' }).click();
  await expect(editor.getByRole('spinbutton', { name: 'Temperature for period 2' })).toHaveValue('20.3');
  await editor.getByRole('button', { name: 'Restore defaults' }).click();
  await expect(editor).not.toBeVisible();
  await expect(valve.getByRole('button', { name: 'Edit schedule', exact: true })).toBeVisible();
  await valve.getByRole('button', { name: 'Edit schedule', exact: true }).click();
  await expect(editor.getByRole('spinbutton', { name: 'Temperature for period 2' })).toHaveValue('21');
  await expect(editor.getByRole('button', { name: 'Restore defaults' })).toHaveCount(0);
});

test('valve periods are edited by start time and can be added and removed', async ({ page }) => {
  await page.goto('/#heating');
  const valve = page.getByRole('article', { name: 'Ensuite', exact: true });
  await valve.getByRole('button', { name: 'Edit schedule' }).click();
  const editor = page.getByRole('dialog', { name: 'Schedule for Ensuite' });
  const periods = editor.getByRole('group', { name: 'Monday periods' });
  await expect(periods.getByText('until 06:00')).toBeVisible();
  await editor.getByLabel('Start of period 2').fill('06:30');
  await expect(periods.getByText('until 06:30')).toBeVisible();
  await editor.getByLabel('Start of period 2').fill('00:00');
  await expect(editor.getByRole('status')).toHaveText('Monday: Each period must start before the next one.');
  await expect(editor.getByRole('button', { name: 'Save schedule' })).toBeDisabled();
  await editor.getByLabel('Start of period 2').fill('06:30');
  await editor.getByRole('button', { name: 'Add period' }).click();
  await expect(editor.getByLabel('Start of period 4')).toHaveValue('23:30');
  await expect(periods.getByText('until 23:30')).toBeVisible();
  await editor.getByRole('button', { name: 'Remove period 4' }).click();
  await expect(editor.getByLabel('Start of period 4')).toHaveCount(0);
  await expect(periods.getByText('until 24:00')).toBeVisible();
  await expect(editor.getByRole('button', { name: 'Remove period 1' })).toBeDisabled();
  await expect(editor.getByRole('button', { name: 'Save schedule' })).toBeEnabled();
  const unit = editor.locator('.period-row').first().locator('.unit-field');
  const unitBox = await unit.boundingBox();
  expect(unitBox).not.toBeNull();
  expect(unitBox!.height).toBeLessThan(44);
});

test('boost controls sit on the left and the schedule button on the right', async ({ page }) => {
  await page.goto('/#heating');
  const valve = page.getByRole('article', { name: 'Ensuite', exact: true });
  const row = await valve.locator('.boost-controls').boundingBox();
  const duration = await valve.getByRole('combobox', { name: 'Boost duration for Ensuite' }).boundingBox();
  const boost = await valve.getByRole('button', { name: 'Boost Ensuite', exact: true }).boundingBox();
  const edit = await valve.getByRole('button', { name: 'Edit schedule' }).boundingBox();
  expect(row && duration && boost && edit).toBeTruthy();
  expect(Math.abs(boost!.y - edit!.y)).toBeLessThan(3);
  expect(boost!.x - (duration!.x + duration!.width)).toBeLessThan(16);
  expect(boost!.x + boost!.width).toBeLessThan(row!.x + row!.width / 2);
  expect(Math.abs(edit!.x + edit!.width - (row!.x + row!.width))).toBeLessThan(2);
  await expect(valve.getByText('Boost uses the valve’s heat demand. Pump and open-window protection still apply.')).toHaveCount(0);
});

test('schedule popup fields follow the dark theme', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/#heating');
  await page.getByRole('article', { name: 'Ensuite', exact: true }).getByRole('button', { name: 'Edit schedule' }).click();
  const editor = page.getByRole('dialog', { name: 'Schedule for Ensuite' });
  await expect(editor).toHaveCSS('background-color', 'rgb(26, 36, 33)');
  await expect(editor.getByRole('spinbutton', { name: 'Temperature for period 2' })).toHaveCSS('background-color', 'rgb(21, 30, 27)');
  await expect(editor.getByLabel('Start of period 2')).toHaveCSS('background-color', 'rgb(21, 30, 27)');
  await expect(editor.getByRole('tab', { name: 'Monday' })).toHaveCSS('background-color', 'rgb(33, 59, 50)');
});
