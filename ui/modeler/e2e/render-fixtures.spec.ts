import { readFileSync, readdirSync } from 'node:fs';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { expect, test } from '@playwright/test';

interface RenderFixture {
  schemaVersion: string;
  model: {
    locationMap: Record<string, unknown>;
    processes: Array<{
      flowElements?: RenderFlowElement[];
    }>;
  };
  [key: string]: unknown;
}

interface RenderFlowElement {
  elementType: string;
  flowElements?: RenderFlowElement[];
}

interface ModelerTestWindow extends Window {
  __FLOWABLE_MODELER_TEST__?: {
    setDocument: (document: RenderFixture) => void;
  };
}

const fixtureDirectory = fileURLToPath(new URL('./fixtures', import.meta.url));
const fixtures = readdirSync(fixtureDirectory)
  .filter((name) => name.endsWith('.json'))
  .sort();

test.describe('representative BPMN renderer fixtures', () => {
  for (const fixture of fixtures) {
    test(`renders ${fixture}`, async ({ page }) => {
      const document = JSON.parse(
        readFileSync(join(fixtureDirectory, fixture), 'utf8'),
      ) as RenderFixture;
      // Harness is mounted on the BPMN workspace; sample id stays offline.
      await page.goto('./models/sample/bpmn');
      await page.evaluate((nextDocument) => {
        const harness = (window as ModelerTestWindow).__FLOWABLE_MODELER_TEST__;
        if (!harness) throw new Error('Modeler E2E harness is unavailable');
        harness.setDocument(nextDocument);
      }, document);

      const expectedNodes = document.model.processes.flatMap((process) =>
        flattenFlowElements(process.flowElements ?? []).filter(
          (element) => element.elementType !== 'sequenceFlow',
        ),
      );
      await expect(page.locator('.diagram-element')).toHaveCount(expectedNodes.length);
      await expect(page.locator('.diagram-element, .pool-shape').first()).toBeVisible();
      await expect(page.getByText(`Protocol ${document.schemaVersion}`)).toBeVisible();
      await expect(
        page.getByText(`${Object.keys(document.model.locationMap).length} DI bounds`),
      ).toBeVisible();
      await page.getByRole('button', { name: 'Fit', exact: true }).click();
      await expect(page.locator('.canvas-coordinate')).not.toHaveText('82% · x 16 · y 18');
      await expect(page).toHaveScreenshot(`${basename(fixture, '.json')}.png`, {
        animations: 'disabled',
        caret: 'hide',
        fullPage: true,
      });
    });
  }
});

function flattenFlowElements(elements: RenderFlowElement[]): RenderFlowElement[] {
  return elements.flatMap((element) => [
    element,
    ...flattenFlowElements(element.flowElements ?? []),
  ]);
}
