import { readFileSync } from 'node:fs';

import { expect, test } from '@playwright/test';

interface TestFormDocument {
  model: {
    name?: string | null;
    fields?: Array<{ id?: string | null; name?: string | null; type?: string | null }>;
    outcomes?: Array<{ id?: string | null; name?: string | null }>;
  };
  [key: string]: unknown;
}

const fixtureUrl = new URL('./fixtures/form/leave-request-form.json', import.meta.url);

test('edits form fields through the UI and survives a save round-trip', async ({ page }) => {
  let serverDocument = JSON.parse(readFileSync(fixtureUrl, 'utf8')) as TestFormDocument;
  let putCount = 0;

  await page.route('**/modeler-app/rest/form-models/e2e-leave-form/editor/form-json', async (route) => {
    if (route.request().method() === 'PUT') {
      const payload: unknown = route.request().postDataJSON();
      serverDocument = payload as TestFormDocument;
      serverDocument.model.name = 'Server-normalized leave form';
      putCount += 1;
      await route.fulfill({ status: 204 });
      return;
    }
    await route.fulfill({ status: 200, json: serverDocument });
  });

  await page.goto('./models/e2e-leave-form/form');
  await expect(page.getByRole('button', { name: 'Save' })).toBeEnabled();
  await expect(page.getByRole('list', { name: 'Form field list' })).toBeVisible();
  await expect(page.locator('[data-field-id="employeeName"]')).toBeVisible();
  await expect(page.getByRole('link', { name: 'Back to list' })).toBeVisible();

  // Palette adds a field through the command stack.
  await page.getByRole('button', { name: 'Text', exact: true }).click();
  await expect(page.locator('[data-field-id="text1"]')).toBeVisible();
  await expect(page.getByText('Local changes')).toBeVisible();
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect(page.locator('[data-field-id="text1"]')).toHaveCount(0);
  await page.getByRole('button', { name: 'Redo' }).click();
  await expect(page.locator('[data-field-id="text1"]')).toBeVisible();

  // Property edits commit on blur.
  await page.getByRole('button', { name: 'Select field employeeName' }).click();
  await page.getByLabel('Label', { exact: true }).fill('Full name');
  await page.getByLabel('Label', { exact: true }).blur();
  await expect(page.locator('[data-field-id="employeeName"]')).toContainText('Full name');

  // Preview mode renders submittable fields and outcomes.
  await page.getByRole('button', { name: 'Preview' }).click();
  await expect(page.getByRole('form', { name: 'Form preview' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Submit request' })).toBeVisible();
  await page.getByRole('button', { name: 'Design' }).click();

  await page.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText('Saved and reloaded from the server', { exact: true })).toBeVisible();
  expect(putCount).toBe(1);
  expect(serverDocument.model.fields?.some((field) => field.id === 'text1')).toBe(true);
  expect(
    serverDocument.model.fields?.find((field) => field.id === 'employeeName')?.name,
  ).toBe('Full name');
  await expect(page.locator('.model-title-block strong')).toHaveText(
    'Server-normalized leave form',
  );

  await page.reload();
  await expect(page.locator('[data-field-id="text1"]')).toBeVisible();
  await expect(page.locator('[data-field-id="employeeName"]')).toContainText('Full name');
  await expect(page.locator('.model-title-block strong')).toHaveText(
    'Server-normalized leave form',
  );
});
