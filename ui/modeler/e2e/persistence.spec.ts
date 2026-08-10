import { readFileSync } from 'node:fs';

import { expect, test } from '@playwright/test';

interface TestBpmnDocument {
  model: {
    processes: Array<{ name?: string | null }>;
  };
  [key: string]: unknown;
}

const fixtureUrl = new URL('./fixtures/01-simplemodel-bpmn.json', import.meta.url);

test('saves canonical JSON through the UI boundary and survives a browser reload', async ({
  page,
}) => {
  let serverDocument = JSON.parse(readFileSync(fixtureUrl, 'utf8')) as TestBpmnDocument;
  let putCount = 0;

  await page.route('**/modeler-app/rest/models/e2e-leave/editor/bpmn-json', async (route) => {
    if (route.request().method() === 'PUT') {
      const payload: unknown = route.request().postDataJSON();
      serverDocument = payload as TestBpmnDocument;
      const process = serverDocument.model.processes[0];
      if (!process) throw new Error('saved BPMN document must contain a process');
      process.name = 'Server-normalized leave process';
      putCount += 1;
      await route.fulfill({ status: 204 });
      return;
    }
    await route.fulfill({ status: 200, json: serverDocument });
  });

  await page.goto('./models/e2e-leave/bpmn');
  await expect(page.getByRole('button', { name: 'Save' })).toBeEnabled();
  await page.getByRole('button', { name: 'User task', exact: true }).click();
  await expect(page.locator('[data-element-id="modeler-userTask-1"]')).toBeVisible();

  await page.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText('Saved and reloaded from the server', { exact: true })).toBeVisible();
  expect(putCount).toBe(1);
  await expect(
    page.getByRole('heading', { name: 'Server-normalized leave process' }),
  ).toBeVisible();

  await page.reload();
  await expect(page.locator('[data-element-id="modeler-userTask-1"]')).toBeVisible();
  await expect(
    page.getByRole('heading', { name: 'Server-normalized leave process' }),
  ).toBeVisible();
});
