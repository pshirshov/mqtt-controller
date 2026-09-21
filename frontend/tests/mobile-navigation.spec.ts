import { expect, test } from '@playwright/test';
import { snapshotSchema } from '../src/protocol';

test.use({ hasTouch: true });

// Specified: mobile tabs open navigation popups instead of navigating immediately.
for (const width of [320, 390]) {
  test(`mobile navigation exposes aggregate views at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 740 });
    await page.goto('/#lights');
    for (const name of ['Plugs', 'Heating']) {
      const before = page.url();
      const navigation = page.getByRole('navigation', { name: /^(Main|Mobile) navigation$/ });
      await navigation.getByText(name, { exact: true }).tap();
      const popup = page.getByRole('dialog', { name: `${name} navigation` });
      await expect(popup).toBeVisible();
      expect(page.url()).toBe(before);
      await expect(popup.getByRole('link').first()).toHaveText(`Energy · ${name}`);
      await page.screenshot({ path: `test-results/mobile-menu-${name.toLowerCase()}-${width}.png` });
      await popup.getByRole('link', { name: `Energy · ${name}`, exact: true }).tap();
      await expect(popup).not.toBeVisible();
      await expect(page.getByRole('heading', { name: `Energy · ${name}`, exact: true })).toBeVisible();
      await expect(page).toHaveURL(new RegExp(`#energy/${name.toLowerCase()}$`));
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  });
}

test('mobile navigation dismisses with Escape, close and backdrop and restores focus', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 740 });
  await page.goto('/#lights');
  const tab = page.getByRole('navigation', { name: 'Mobile navigation' }).getByRole('button', { name: 'Lights', exact: true });
  const popup = page.getByRole('dialog', { name: 'Lights navigation' });
  await tab.focus();
  await page.keyboard.press('Enter');
  await expect(popup).toBeVisible();
  await expect(tab).toHaveAttribute('aria-expanded', 'true');
  await page.keyboard.press('Escape');
  await expect(popup).not.toBeVisible();
  await expect(tab).toBeFocused();
  await expect(tab).toHaveAttribute('aria-expanded', 'false');
  await tab.click();
  await popup.getByRole('button', { name: 'Close navigation' }).click();
  await expect(popup).not.toBeVisible();
  await tab.click();
  await page.mouse.click(2, 730);
  await expect(popup).not.toBeVisible();
  await expect(page).toHaveURL(/#lights$/);
});

test('mobile navigation closes when resized to the desktop sidebar', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 740 });
  await page.goto('/#lights');
  await page.getByRole('navigation', { name: 'Mobile navigation' }).getByRole('button', { name: 'Heating', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.setViewportSize({ width: 1280, height: 900 });
  await expect(page.getByRole('dialog')).not.toBeVisible();
  await expect(page.getByRole('navigation', { name: 'Mobile navigation' })).not.toBeVisible();
  await page.getByRole('link', { name: 'Energy: Heating', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Energy · Heating', exact: true })).toBeVisible();
});

test('mobile navigation scrolls long room lists and selects sections on another page', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.setViewportSize({ width: 390, height: 400 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.routeWebSocket('**/ws', socket => {
    const server = socket.connectToServer();
    server.onMessage(data => {
      const parsed = snapshotSchema.safeParse(JSON.parse(data.toString()));
      if (!parsed.success) { socket.send(data); return; }
      socket.send(JSON.stringify({ ...parsed.data, plugs: Array.from({ length: 30 }, (_, index) => ({
        ...parsed.data.plugs[0], device: `plug-${index}`, room: `room-${index}`,
      })) }));
    });
  });
  await page.goto('/#lights');
  await page.getByRole('searchbox').fill('missing');
  await page.getByRole('navigation', { name: 'Mobile navigation' }).getByRole('button', { name: 'Plugs', exact: true }).click();
  const popup = page.getByRole('dialog', { name: 'Plugs navigation' });
  await expect(popup.getByRole('group').getByRole('button')).toHaveCount(30);
  const lastRoom = popup.getByRole('button', { name: 'Room 29', exact: true });
  await lastRoom.scrollIntoViewIfNeeded();
  await expect(lastRoom).toBeInViewport();
  const bounds = await popup.boundingBox();
  if (bounds === null) throw new Error('Missing popup bounds');
  expect(bounds.y).toBeGreaterThanOrEqual(0);
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(400);
  await page.screenshot({ path: 'test-results/mobile-navigation-long-list.png' });
  await lastRoom.click();
  await expect(popup).not.toBeVisible();
  await expect(page).toHaveURL(/#plugs$/);
  await expect(page.getByRole('searchbox')).toHaveValue('');
  await expect(page.getByRole('region', { name: 'Room 29', exact: true }).getByRole('heading', { name: 'Room 29', exact: true })).toBeInViewport();
  expect(errors).toEqual([]);
});

test('mobile heating sections remain in the aggregate view', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 740 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/#energy/heating');
  const tab = page.getByRole('navigation', { name: 'Mobile navigation' }).getByRole('button', { name: 'Heating', exact: true });
  await tab.click();
  await page.getByRole('dialog').getByRole('button', { name: 'First floor', exact: true }).click();
  await expect(page).toHaveURL(/#energy\/heating$/);
  await expect(page.getByRole('region', { name: 'First floor', exact: true }).getByRole('heading', { name: 'First floor', exact: true })).toBeInViewport();
  await tab.click();
  await page.getByRole('dialog').getByRole('button', { name: 'Heat pump', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Heat pump', exact: true }).getByRole('heading', { name: 'Heat pump', exact: true })).toBeInViewport();
  await tab.click();
  await page.getByRole('dialog').getByRole('link', { name: 'All heating', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Heating', exact: true })).toBeVisible();
});
