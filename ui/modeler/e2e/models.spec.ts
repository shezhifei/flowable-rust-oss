import { expect, test } from '@playwright/test';

test('lists models and navigates into a newly created form editor', async ({ page }) => {
  const models = [
    {
      id: 'bpmn-1',
      name: 'Leave process',
      key: 'leaveProcess',
      category: null,
      version: 1,
      lastUpdateTime: '2026-08-01T10:00:00.000Z',
      createTime: '2026-08-01T09:00:00.000Z',
    },
  ];

  await page.route('**/repository/models**', async (route) => {
    const method = route.request().method();
    const url = new URL(route.request().url());

    if (method === 'GET' && !url.pathname.endsWith('/source') && !url.pathname.match(/\/models\/[^/]+$/)) {
      await route.fulfill({
        status: 200,
        json: { data: models, total: models.length, start: 0, size: 1000 },
      });
      return;
    }

    if (method === 'POST' && url.pathname.endsWith('/models')) {
      const body = route.request().postDataJSON() as { name: string; key: string };
      const created = {
        id: 'form-created-1',
        name: body.name,
        key: body.key,
        category: null,
        version: 1,
        lastUpdateTime: '2026-08-10T12:00:00.000Z',
        createTime: '2026-08-10T12:00:00.000Z',
      };
      models.push(created);
      await route.fulfill({ status: 201, json: created });
      return;
    }

    await route.continue();
  });

  await page.route('**/repository/models/*/source', async (route) => {
    const url = route.request().url();
    if (route.request().method() === 'GET') {
      if (url.includes('bpmn-1')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/xml',
          body: '<?xml version="1.0"?><definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"><process id="leaveProcess"/></definitions>',
        });
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          schemaVersion: '1.0',
          model: { key: 'newForm', name: 'New form', fields: [], outcomes: [] },
        }),
      });
      return;
    }
    if (route.request().method() === 'PUT') {
      await route.fulfill({ status: 204 });
      return;
    }
    await route.continue();
  });

  await page.route(
    '**/modeler-app/rest/form-models/form-created-1/editor/form-json',
    async (route) => {
      await route.fulfill({
        status: 200,
        json: {
          schemaVersion: '1.0',
          model: { key: 'intakeForm', name: 'Intake form', fields: [], outcomes: [] },
        },
      });
    },
  );

  await page.goto('./');
  await expect(page.getByRole('heading', { name: 'Models' })).toBeVisible();
  await expect(page.getByRole('table', { name: 'Model list table' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Leave process', exact: true })).toBeVisible();
  await expect(page.getByText('BPMN', { exact: true })).toBeVisible();

  await page.getByRole('button', { name: '+ Form' }).click();
  await page.getByLabel('Model name').fill('Intake form');
  await page.getByLabel('Model key').fill('intakeForm');
  await page.getByRole('button', { name: 'Create' }).click();

  await expect(page).toHaveURL(/\/models\/form-created-1\/form$/);
  await expect(page.getByRole('button', { name: 'Save' })).toBeEnabled();
  await expect(page.locator('.model-title-block strong')).toHaveText('Intake form');
  await expect(page.getByRole('complementary', { name: 'Form field palette' })).toBeVisible();
});
