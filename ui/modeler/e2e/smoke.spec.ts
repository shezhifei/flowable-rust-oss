import { expect, test } from '@playwright/test';

test('renders and navigates the typed BPMN canvas at the mounted base path', async ({ page }) => {
  // Home is the model repository; the offline sample process lives under /models/sample/bpmn.
  await page.goto('./models/sample/bpmn');

  await expect(page).toHaveTitle('Flowable Modeler');
  await expect(page.getByRole('application', { name: 'BPMN process canvas' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Review request' })).toBeVisible();
  await expect(page.getByText('Protocol 1.0')).toBeVisible();
  await expect(page.getByRole('link', { name: 'Back to list' })).toBeVisible();
  await expect(page.locator('[data-element-id="review"]')).toHaveClass(/is-selected/);

  const review = page.locator('[data-element-id="review"]');
  const reviewSurface = review.locator('rect').first();
  const originalX = Number(await reviewSurface.getAttribute('x'));
  const bounds = await review.boundingBox();
  if (!bounds) throw new Error('review task must have browser bounds');
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width / 2 + 64, bounds.y + bounds.height / 2 + 32, {
    steps: 4,
  });
  await page.mouse.up();

  const movedX = Number(await reviewSurface.getAttribute('x'));
  expect(movedX).toBeGreaterThan(originalX + 50);
  await expect(page.getByText('Local changes')).toBeVisible();
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect(reviewSurface).toHaveAttribute('x', String(originalX));
  await page.keyboard.press('Control+Shift+z');
  expect(Number(await reviewSurface.getAttribute('x'))).toBeCloseTo(movedX, 5);
  await page.keyboard.press('Control+z');
  await expect(reviewSurface).toHaveAttribute('x', String(originalX));
  await page.getByRole('button', { name: 'Redo' }).click();
  expect(Number(await reviewSurface.getAttribute('x'))).toBeCloseTo(movedX, 5);

  await page.locator('[data-element-id="notify"]').click();
  await expect(page.getByRole('heading', { name: 'Notify employee' })).toBeVisible();
  await expect(page.locator('[data-element-id="notify"]')).toHaveClass(/is-selected/);

  await page.getByRole('button', { name: 'Zoom in' }).click();
  await expect(page.getByLabel('Zoom level')).toHaveText('90%');

  await page.getByRole('button', { name: 'User task', exact: true }).click();
  await expect(page.locator('[data-element-id="modeler-userTask-1"]')).toBeVisible();
  await expect(page.locator('[data-element-id="modeler-userTask-1"]')).toHaveClass(/is-selected/);
  await expect(page.getByRole('heading', { name: 'User task' })).toBeVisible();
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect(page.locator('[data-element-id="modeler-userTask-1"]')).toHaveCount(0);
  await page.getByRole('button', { name: 'Redo' }).click();
  await expect(page.locator('[data-element-id="modeler-userTask-1"]')).toBeVisible();
  await page.keyboard.press('Delete');
  await expect(page.locator('[data-element-id="modeler-userTask-1"]')).toHaveCount(0);
  await page.keyboard.press('Control+z');
  await expect(page.locator('[data-element-id="modeler-userTask-1"]')).toBeVisible();
});
