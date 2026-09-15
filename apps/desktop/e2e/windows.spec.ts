import { expect, test } from '@playwright/test';

// Cheap, stable window-routing checks that mirror the ?window= pattern used
// in home.spec.ts. No screenshots: these surfaces poll live IPC status, so
// pixel snapshots would be flaky by design.
test('editor window renders with no-project copy in browser', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('kiri.theme.mode', 'dark'));
  await page.setViewportSize({ width: 900, height: 640 });
  await page.goto('/?window=editor');
  await expect(page.getByLabel('Kiri editor')).toBeVisible();
  await expect(page.getByText('Kiri Editor')).toBeVisible();
  await expect(page.getByText('No project open').first()).toBeVisible();
});

test('hud overlay renders the idle pill in browser', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('kiri.theme.mode', 'dark'));
  await page.setViewportSize({ width: 280, height: 64 });
  await page.goto('/?window=hud-overlay');
  await expect(page.getByLabel('Recording HUD')).toBeVisible();
  await expect(page.getByLabel('Recording HUD')).toContainText('Kiri');
});

test('standalone countdown renders the idle placeholder in browser', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('kiri.theme.mode', 'dark'));
  await page.setViewportSize({ width: 320, height: 200 });
  await page.goto('/?window=countdown');
  await expect(page.getByLabel('Recording countdown')).toBeVisible();
  await expect(page.getByRole('status')).toContainText('…');
});
