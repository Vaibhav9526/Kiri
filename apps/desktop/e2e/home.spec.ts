import { expect, test } from '@playwright/test';

for (const theme of ['light', 'dark', 'system'] as const) {
  test(`home renders in ${theme} mode`, async ({ page }) => {
    await page.addInitScript((value) => localStorage.setItem('kiri.theme.mode', value), theme);
    await page.setViewportSize({ width: 820, height: 680 });
    await page.goto('/');
    await expect(page.getByRole('heading', { name: 'Create a walkthrough' })).toBeVisible();
    await expect(page.locator('html')).toHaveAttribute('data-theme-mode', theme);
    await expect(page).toHaveScreenshot(`home-${theme}.png`, { animations: 'disabled' });
  });
}

test('project naming surface is compact and keyboard reachable', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('kiri.theme.mode', 'dark'));
  await page.setViewportSize({ width: 820, height: 680 });
  await page.goto('/');
  await page.getByRole('button', { name: /New Kiri Project/ }).click();
  await expect(page.getByLabel('Project name')).toBeFocused();
  await expect(page).toHaveScreenshot('home-project-name-dark.png', { animations: 'disabled' });
});

test('source selector renders as a focused boundary', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('kiri.theme.mode', 'dark'));
  await page.setViewportSize({ width: 760, height: 600 });
  await page.goto('/?window=source-selector');
  await expect(page.getByText('Capture setup')).toBeVisible();
  await expect(page.getByRole('tab', { name: 'Displays' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await expect(page).toHaveScreenshot('source-selector-dark.png', { animations: 'disabled' });
});

test('recording controller boundary is honest when idle', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('kiri.theme.mode', 'dark'));
  await page.setViewportSize({ width: 280, height: 48 });
  await page.goto('/?window=recording-controller');
  await expect(page.getByLabel('Recording controller')).toBeVisible();
  await expect(page.getByText('IDLE')).toBeVisible();
  await expect(page.getByText('No active recording')).toBeVisible();
  await expect(page).toHaveScreenshot('recording-controller-dark.png', { animations: 'disabled' });
});
