import { readFileSync } from 'node:fs';

import { expect, test } from '@playwright/test';

interface TestBpmnDocument {
  model: {
    processes: Array<{ id?: string | null; name?: string | null }>;
    pools: Array<{ id?: string | null; processRef?: string | null }>;
  };
  [key: string]: unknown;
}

interface ModelerTestWindow extends Window {
  __FLOWABLE_MODELER_TEST__?: {
    setDocument: (document: TestBpmnDocument) => void;
    getDocument: () => TestBpmnDocument;
  };
}

// The only fixture with two participants; see the render fixture exporter.
const fixtureUrl = new URL('./fixtures/21-messageflow-bpmn.json', import.meta.url);

test('edits the process of each participant in a collaboration', async ({ page }) => {
  const document = JSON.parse(readFileSync(fixtureUrl, 'utf8')) as TestBpmnDocument;

  await page.goto('./models/sample/bpmn');
  await page.evaluate((nextDocument) => {
    const harness = (window as ModelerTestWindow).__FLOWABLE_MODELER_TEST__;
    if (!harness) throw new Error('Modeler E2E harness is unavailable');
    harness.setDocument(nextDocument);
  }, document);

  // With nothing selected the panel edits the main process and lists both pools.
  await expect(page.locator('[data-panel-state="process"]')).toBeVisible();
  await expect(page.locator('[data-property-group="pools"]')).toContainText('2 participants');

  // Reaching the second participant used to be impossible: the panel wrote every
  // process edit to processes[0].
  await page.locator('[data-pool-target="participant2"]').click();
  await expect(page.locator('[data-panel-state="pool"]')).toBeVisible();
  await expect(page.locator('[data-property="processId"]')).toHaveValue('PROCESS_2');

  const processName = page.locator('[data-property="processName"]');
  await processName.fill('Second participant');
  await processName.blur();

  await expect
    .poll(async () =>
      page.evaluate(() => {
        const harness = (window as ModelerTestWindow).__FLOWABLE_MODELER_TEST__;
        if (!harness) throw new Error('Modeler E2E harness is unavailable');
        return harness.getDocument().model.processes.map((process) => process.name ?? null);
      }),
    )
    .toEqual(['process 1', 'Second participant']);

  // Selecting the first pool reaches the other process through the same panel.
  // A pool's interior is deliberately click-through, so its label is the handle.
  await page.locator('.pool-shape[data-element-id="participant1"] text').click();
  await expect(page.locator('[data-panel-state="pool"]')).toBeVisible();
  await expect(page.locator('[data-property="processId"]')).toHaveValue('PROCESS_1');

  // A process id rename has to drag its pool's processRef along, or the pool
  // would point at a process that no longer exists.
  const processId = page.locator('[data-property="processId"]');
  await processId.fill('MAIN_PROCESS');
  await processId.blur();

  await expect
    .poll(async () =>
      page.evaluate(() => {
        const harness = (window as ModelerTestWindow).__FLOWABLE_MODELER_TEST__;
        if (!harness) throw new Error('Modeler E2E harness is unavailable');
        const model = harness.getDocument().model;
        return {
          processes: model.processes.map((process) => process.id ?? null),
          refs: model.pools.map((pool) => pool.processRef ?? null),
        };
      }),
    )
    .toEqual({
      processes: ['MAIN_PROCESS', 'PROCESS_2'],
      refs: ['MAIN_PROCESS', 'PROCESS_2'],
    });
});
