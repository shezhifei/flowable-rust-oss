import { readFileSync } from 'node:fs';

import { expect, test } from '@playwright/test';

interface TestDmnDocument {
  model: {
    decisions?: Array<{
      id: string;
      name?: string | null;
      decisionTable: {
        hitPolicy: string;
        rules?: Array<{
          inputEntries?: Array<{ text?: string | null }>;
          outputEntries?: Array<{ text?: string | null }>;
        }>;
      };
    }>;
  };
  [key: string]: unknown;
}

const fixtureUrl = new URL('./fixtures/dmn/leave-approval-decision-table.json', import.meta.url);

test('edits decision table cells through the UI and survives a save round-trip', async ({
  page,
}) => {
  let serverDocument = JSON.parse(readFileSync(fixtureUrl, 'utf8')) as TestDmnDocument;
  let putCount = 0;

  await page.route('**/modeler-app/rest/models/e2e-leave-dmn/editor/dmn-json', async (route) => {
    if (route.request().method() === 'PUT') {
      const payload: unknown = route.request().postDataJSON();
      serverDocument = payload as TestDmnDocument;
      const decision = serverDocument.model.decisions?.[0];
      if (!decision) throw new Error('saved DMN document must contain a decision');
      decision.name = 'Server-normalized leave decision';
      putCount += 1;
      await route.fulfill({ status: 204 });
      return;
    }
    await route.fulfill({ status: 200, json: serverDocument });
  });

  await page.goto('./models/e2e-leave-dmn/dmn');
  await expect(page.getByRole('button', { name: 'Save' })).toBeEnabled();
  await expect(page.getByRole('table', { name: 'Decision table Leave approval' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Input column 1' })).toContainText('Leave days');
  await expect(page.getByLabel('output cell 1:1')).toHaveValue('"APPROVED"');

  // A valid cell draft commits on blur and becomes undoable.
  await page.getByLabel('output cell 1:1').fill('"ESCALATED"');
  await page.getByLabel('output cell 1:1').blur();
  await expect(page.getByText('Local changes')).toBeVisible();
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect(page.getByLabel('output cell 1:1')).toHaveValue('"APPROVED"');
  await page.getByRole('button', { name: 'Redo' }).click();
  await expect(page.getByLabel('output cell 1:1')).toHaveValue('"ESCALATED"');

  // An out-of-subset draft is flagged and never enters the document.
  await page.getByLabel('input cell 1:1').fill('leaveDays && role');
  await page.getByLabel('input cell 1:1').blur();
  await expect(page.locator('.dmn-cell.has-error')).toBeVisible();
  await page.getByLabel('input cell 1:1').fill('[3..10]');
  await page.getByLabel('input cell 1:1').blur();
  await expect(page.locator('.dmn-cell.has-error')).toHaveCount(0);
  await expect(page.getByLabel('input cell 1:1')).toHaveValue('[3..10]');

  // Hit policy changes go through undoable commands.
  await page.getByLabel('Hit policy').selectOption('PRIORITY');
  await expect(page.getByLabel('Hit policy')).toHaveValue('PRIORITY');
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect(page.getByLabel('Hit policy')).toHaveValue('FIRST');

  // Column editing opens in the properties panel.
  await page.getByRole('button', { name: 'Output column 1' }).click();
  await expect(page.getByLabel('Name', { exact: true })).toHaveValue('status');
  await page.getByLabel('Name', { exact: true }).fill('approvalStatus');
  await page.getByLabel('Name', { exact: true }).blur();
  await expect(page.getByRole('button', { name: 'Output column 1' })).toContainText(
    'approvalStatus',
  );

  await page.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText('Saved and reloaded from the server', { exact: true })).toBeVisible();
  expect(putCount).toBe(1);
  expect(serverDocument.model.decisions?.[0]?.decisionTable.rules?.[0]?.outputEntries?.[0]?.text).toBe(
    '"ESCALATED"',
  );
  expect(serverDocument.model.decisions?.[0]?.decisionTable.rules?.[0]?.inputEntries?.[0]?.text).toBe(
    '[3..10]',
  );
  await expect(page.locator('.model-title-block strong')).toHaveText(
    'Server-normalized leave decision',
  );

  await page.reload();
  await expect(page.getByLabel('output cell 1:1')).toHaveValue('"ESCALATED"');
  await expect(page.getByLabel('input cell 1:1')).toHaveValue('[3..10]');
  await expect(
    page.getByRole('heading', { name: 'Server-normalized leave decision' }),
  ).toBeVisible();
});
