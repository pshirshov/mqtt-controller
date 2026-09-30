import { expect, test } from '@playwright/test';

test('light group schedule editor saves slot overrides and restores defaults', async ({ page }) => {
  await page.goto('/#lights');
  const room = page.getByRole('article', { name: 'Ensuite', exact: true });
  await expect(room.getByRole('button', { name: 'Recall scene 3 in Ensuite' })).toBeVisible();
  await room.getByRole('button', { name: 'Edit light schedule' }).click();
  const editor = page.getByRole('dialog', { name: 'Light schedule for Ensuite' });
  await expect(editor.getByRole('textbox', { name: 'Evening start' })).toHaveValue('18:00');
  await editor.getByRole('textbox', { name: 'Evening scenes' }).fill('2, 9');
  await expect(editor.getByRole('button', { name: 'Save schedule' })).toBeDisabled();
  await editor.getByRole('textbox', { name: 'Evening end' }).fill('sunset+01:00');
  await editor.getByRole('textbox', { name: 'Night start' }).fill('sunset+01:00');
  await editor.getByRole('textbox', { name: 'Evening scenes' }).fill('2, 1');
  await editor.getByRole('button', { name: 'Save schedule' }).click();
  await expect(editor).not.toBeVisible();
  await expect(room.getByRole('button', { name: /Edit light schedule · Override active/ })).toBeVisible();
  await expect(room.getByRole('button', { name: 'Recall scene 3 in Ensuite' })).toHaveCount(0);
  await expect(room.getByRole('status')).toHaveCount(0);

  await room.getByRole('button', { name: 'Restore default light schedule for Ensuite' }).click();
  await expect(room.getByRole('button', { name: 'Edit light schedule', exact: true })).toBeVisible();
  await expect(room.getByRole('button', { name: 'Recall scene 3 in Ensuite' })).toBeVisible();
});

test('motion schedule editor requires scenes for every slot', async ({ page }) => {
  await page.goto('/#lights');
  const room = page.getByRole('article', { name: 'Ensuite', exact: true });
  await room.getByText('Lights & automation').click();
  await room.getByRole('button', { name: 'Edit motion schedule' }).click();
  const editor = page.getByRole('dialog', { name: 'Motion schedule for Ensuite motion' });
  await editor.getByRole('textbox', { name: 'Day scenes' }).fill('');
  await expect(editor.getByRole('button', { name: 'Save schedule' })).toBeDisabled();
  await editor.getByRole('textbox', { name: 'Day scenes' }).fill('2');
  await editor.getByRole('button', { name: 'Save schedule' }).click();
  await expect(room.getByRole('button', { name: /Edit motion schedule · Override active/ })).toBeVisible();
});

test('timed actions and kill switches accept overrides', async ({ page }) => {
  await page.goto('/#lights');
  const room = page.getByRole('article', { name: 'Ensuite', exact: true });
  await room.getByText('Lights & automation').click();
  const time = room.getByRole('textbox', { name: 'Time for Ensuite night off' });
  await time.fill('25:00h');
  await expect(room.getByRole('form', { name: 'Timed action Ensuite night off' }).getByRole('button', { name: 'Save' })).toBeDisabled();
  await time.fill('sunset-00:30');
  await room.getByRole('form', { name: 'Timed action Ensuite night off' }).getByRole('button', { name: 'Save' }).click();
  await expect(room.getByText('turn_off → ensuite · Override')).toBeVisible();
  await room.getByRole('button', { name: 'Restore default' }).click();
  await expect(room.getByRole('textbox', { name: 'Time for Ensuite night off' })).toHaveValue('23:30');

  await page.goto('/#plugs');
  const plug = page.getByRole('article', { name: '3d printer' });
  await plug.getByText('Automation & device').click();
  await plug.getByRole('spinbutton', { name: 'Threshold for Printer idle' }).fill('2.5');
  await plug.getByRole('spinbutton', { name: 'Holdoff for Printer idle' }).fill('45');
  await plug.getByRole('form', { name: 'Kill switch Printer idle' }).getByRole('button', { name: 'Save' }).click();
  await expect(plug.getByText('Turns off below 2.5 W for 45s · Override.')).toBeVisible();
  await plug.getByRole('form', { name: 'Kill switch Printer idle' }).getByRole('button', { name: 'Restore default' }).click();
  await expect(plug.getByText('Turns off below 10 W for 2m.')).toBeVisible();
  await plug.getByRole('textbox', { name: 'Time for Printer morning' }).fill('07:45');
  await plug.getByRole('form', { name: 'Timed action Printer morning' }).getByRole('button', { name: 'Save' }).click();
  await expect(plug.getByText('turn_on → sonoff-p-printer · Override')).toBeVisible();
});
