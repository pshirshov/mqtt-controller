import { expect, test } from '@playwright/test';

test('valve schedule editor saves a weekly override and resets it', async ({ page }) => {
  await page.goto('/#heating');
  const valve = page.getByRole('article', { name: 'Ensuite', exact: true });
  await valve.getByRole('button', { name: 'Edit schedule' }).click();
  const editor = page.getByRole('dialog', { name: 'Schedule for Ensuite' });
  await expect(editor).toBeVisible();
  await editor.getByRole('spinbutton', { name: 'Temperature for period 2' }).fill('20.3');
  await editor.getByRole('button', { name: 'Copy this day to all days' }).click();
  await editor.getByRole('button', { name: 'Save schedule' }).click();
  await expect(editor).not.toBeVisible();
  await expect(valve.getByRole('button', { name: /Override active/ })).toBeVisible();
  await expect(valve.getByRole('button', { name: 'Restore default schedule' })).toBeVisible();

  await valve.getByRole('button', { name: /Override active/ }).click();
  await editor.getByRole('combobox', { name: 'Day' }).selectOption('friday');
  await expect(editor.getByRole('spinbutton', { name: 'Temperature for period 2' })).toHaveValue('20.3');
  await editor.getByRole('button', { name: 'Close schedule editor' }).click();
  await valve.getByRole('button', { name: 'Restore default schedule' }).click();
  await expect(valve.getByRole('button', { name: 'Edit schedule', exact: true })).toBeVisible();
  await valve.getByRole('button', { name: 'Edit schedule', exact: true }).click();
  await expect(editor.getByRole('spinbutton', { name: 'Temperature for period 2' })).toHaveValue('21');
});

test('schedule controls share the boost row without redundant help text', async ({ page }) => {
  await page.goto('/#heating');
  const valve = page.getByRole('article', { name: 'Ensuite', exact: true });
  const boost = await valve.getByRole('button', { name: 'Boost Ensuite', exact: true }).boundingBox();
  const edit = await valve.getByRole('button', { name: 'Edit schedule' }).boundingBox();
  expect(boost).not.toBeNull();
  expect(edit).not.toBeNull();
  expect(Math.abs(boost!.y - edit!.y)).toBeLessThan(3);
  await expect(valve.getByText('Boost uses the valve’s heat demand. Pump and open-window protection still apply.')).toHaveCount(0);
});

test('schedule popup fields follow the dark theme', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/#heating');
  await page.getByRole('article', { name: 'Ensuite', exact: true }).getByRole('button', { name: 'Edit schedule' }).click();
  const editor = page.getByRole('dialog', { name: 'Schedule for Ensuite' });
  await expect(editor).toHaveCSS('background-color', 'rgb(26, 36, 33)');
  await expect(editor.getByRole('spinbutton', { name: 'Temperature for period 2' })).toHaveCSS('background-color', 'rgb(21, 30, 27)');
  await expect(editor.getByRole('combobox', { name: 'Day' })).toHaveCSS('background-color', 'rgb(21, 30, 27)');
});

test('valve schedule editor requires continuous periods', async ({ page }) => {
  await page.goto('/#heating');
  const valve = page.getByRole('article', { name: 'Ensuite', exact: true });
  await valve.getByRole('button', { name: 'Edit schedule' }).click();
  const editor = page.getByRole('dialog', { name: 'Schedule for Ensuite' });
  await editor.getByRole('textbox', { name: 'End of period 1' }).fill('23:30');
  await expect(editor.getByRole('button', { name: 'Save schedule' })).toBeDisabled();
});
