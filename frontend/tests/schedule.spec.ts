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

  await valve.getByRole('button', { name: /Override active/ }).click();
  await editor.getByRole('combobox', { name: 'Day' }).selectOption('friday');
  await expect(editor.getByRole('spinbutton', { name: 'Temperature for period 2' })).toHaveValue('20.3');
  await editor.getByRole('button', { name: 'Use deployed schedule' }).click();
  await expect(valve.getByRole('button', { name: 'Edit schedule', exact: true })).toBeVisible();
});

test('valve schedule editor requires continuous periods', async ({ page }) => {
  await page.goto('/#heating');
  const valve = page.getByRole('article', { name: 'Ensuite', exact: true });
  await valve.getByRole('button', { name: 'Edit schedule' }).click();
  const editor = page.getByRole('dialog', { name: 'Schedule for Ensuite' });
  await editor.getByRole('textbox', { name: 'End of period 1' }).fill('23:30');
  await expect(editor.getByRole('button', { name: 'Save schedule' })).toBeDisabled();
});
