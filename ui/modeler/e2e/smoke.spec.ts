import { expect, test } from '@playwright/test';

test('renders and navigates the typed BPMN canvas at the mounted base path', async ({ page }) => {
  await page.goto('./');

  await expect(page).toHaveTitle('Flowable Modeler');
  await expect(page.getByRole('application', { name: 'BPMN process canvas' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Review request' })).toBeVisible();
  await expect(page.getByText('Protocol 1.0')).toBeVisible();
  await expect(page.locator('[data-element-id="review"]')).toHaveClass(/is-selected/);

  await page.locator('[data-element-id="notify"]').click();
  await expect(page.getByRole('heading', { name: 'Notify employee' })).toBeVisible();
  await expect(page.locator('[data-element-id="notify"]')).toHaveClass(/is-selected/);

  await page.getByRole('button', { name: 'Zoom in' }).click();
  await expect(page.getByLabel('Zoom level')).toHaveText('90%');
});
