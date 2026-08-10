import { expect, test } from '@playwright/test';

import { ADMIN_PASSWORD, ADMIN_USER, BASE_URL, engineGetJson, login } from '../helpers/server';

/**
 * L1 设计: log in through the idm login page, model a form and a BPMN
 * process (user task referencing the form) in the first-party modeler, and
 * publish both as real deployments. Everything runs against the live server
 * with enforced auth; the only REST usage is the final read-only backstop.
 */
test.describe('L1 design', () => {
  test('login, model a form and a process, publish both', async ({ page }) => {
    const suffix = Math.random().toString(36).slice(2, 8);
    const formKey = `l1-form-${suffix}`;
    const processKey = `l1-process-${suffix}`;

    // --- Login through the real login page (cookie session) ---
    await page.goto('/idm/#/login');
    await expect(page.locator('#username')).toBeVisible();
    await page.locator('#username').fill(ADMIN_USER);
    await page.locator('#password').fill(ADMIN_PASSWORD);
    await page.locator('.login-button').click();
    // A confirmed login lands on the task landing page at the root.
    await expect(page).toHaveURL(new RegExp(`${BASE_URL}/#/?$`));

    // --- Create the form model and add a field in the designer ---
    await page.goto('/modeler-app/');
    await expect(page.getByRole('heading', { name: 'Models' })).toBeVisible();

    await page.getByRole('button', { name: '+ Form' }).click();
    await page.getByLabel('Model name').fill(`L1 form ${suffix}`);
    await page.getByLabel('Model key').fill(formKey);
    await page.getByRole('button', { name: 'Create', exact: true }).click();

    await expect(page).toHaveURL(new RegExp(`/modeler-app/models/[^/]+/form$`));
    await expect(page.locator('aside[aria-label="Form field palette"]')).toBeVisible();
    await page.locator('button[data-field-type="text"]').click();
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page.locator('.modeler-notice[role="status"]')).toHaveText(/Saved/);

    // --- Create the BPMN model: user task referencing the form ---
    await page.goto('/modeler-app/');
    await page.getByRole('button', { name: '+ BPMN' }).click();
    await page.getByLabel('Model name').fill(`L1 process ${suffix}`);
    await page.getByLabel('Model key').fill(processKey);
    await page.getByRole('button', { name: 'Create', exact: true }).click();

    await expect(page).toHaveURL(new RegExp(`/modeler-app/models/[^/]+/bpmn$`));
    await expect(page.locator('aside[aria-label="BPMN element palette"]')).toBeVisible();
    await page.locator('button[data-palette-kind="userTask"]').click();

    // The new task is selected after placement; point its form key at the form.
    const formKeyInput = page.locator('input[aria-label="Form key"]');
    await expect(formKeyInput).toBeVisible();
    await formKeyInput.fill(formKey);
    await formKeyInput.blur();

    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page.locator('.modeler-notice[role="status"]')).toHaveText(/Saved/);

    // --- Publish both models from the repository list ---
    await page.goto('/modeler-app/');
    const formRow = page.locator('tr', { has: page.locator('td.model-list-key', { hasText: formKey }) });
    await expect(formRow).toBeVisible();
    await formRow.getByRole('button', { name: 'Publish' }).click();
    await expect(page.locator('.modeler-notice[role="status"]')).toHaveText(/Published/);

    const processRow = page.locator('tr', {
      has: page.locator('td.model-list-key', { hasText: processKey }),
    });
    await expect(processRow).toBeVisible();
    await processRow.getByRole('button', { name: 'Publish' }).click();
    await expect(page.locator('.modeler-notice[role="status"]')).toHaveText(/Published/);

    // --- REST backstop: the deployments produced real definitions ---
    const processDefinitions = await engineGetJson<{ total: number; data: Array<{ key: string }> }>(
      page,
      `/repository/process-definitions?key=${processKey}`,
    );
    expect(processDefinitions.status).toBe(200);
    expect(processDefinitions.body.total).toBeGreaterThanOrEqual(1);

    const formDefinitions = await engineGetJson<{ total: number; data: Array<{ key: string }> }>(
      page,
      `/form-repository/form-definitions?key=${formKey}`,
    );
    expect(formDefinitions.status).toBe(200);
    expect(formDefinitions.body.total).toBeGreaterThanOrEqual(1);
  });
});
