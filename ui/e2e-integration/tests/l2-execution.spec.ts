import { expect, test } from '@playwright/test';

import {
  ADMIN_PASSWORD,
  ADMIN_USER,
  BASE_URL,
  engineGetJson,
  enginePostJson,
  login,
} from '../helpers/server';

function bpmn(processKey: string, processName: string, taskName: string, formKey: string): string {
  return `<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="http://flowable.org/bpmn">
  <process id="${processKey}" name="${processName}" isExecutable="true">
    <startEvent id="start"/>
    <sequenceFlow id="flow-start" sourceRef="start" targetRef="review"/>
    <userTask id="review" name="${taskName}" flowable:assignee="${ADMIN_USER}" flowable:formKey="${formKey}"/>
    <sequenceFlow id="flow-end" sourceRef="review" targetRef="end"/>
    <endEvent id="end"/>
  </process>
</definitions>`;
}

/**
 * L2 执行: start a deployed process from the task app, see the task, open it,
 * render its form, and complete it. Test data is deployed through the engine
 * REST API (Basic) as setup; the flow under test is entirely the task UI.
 */
test.describe('L2 execution', () => {
  test('start process, render task form, complete task', async ({ page }) => {
    const suffix = Math.random().toString(36).slice(2, 8);
    const formKey = `l2-form-${suffix}`;
    const processKey = `l2-process-${suffix}`;
    const processName = `L2 process ${suffix}`;
    const taskName = `L2 review ${suffix}`;

    // --- Setup: deploy the form and the process (engine REST, Basic) ---
    const formDeploy = await enginePostJson(page, '/form-repository/deployments', {
      name: `L2 form ${suffix}`,
      resourceName: `${formKey}.form`,
      resource: JSON.stringify({
        key: formKey,
        name: `L2 form ${suffix}`,
        fields: [{ id: 'comment', name: 'Comment', type: 'text', required: false }],
        outcomes: [],
      }),
    });
    expect(formDeploy.status, JSON.stringify(formDeploy.body)).toBe(201);

    const basicAuth = Buffer.from(`${ADMIN_USER}:${ADMIN_PASSWORD}`).toString('base64');
    const processDeploy = await page.request.post(`${BASE_URL}/repository/deployments`, {
      headers: { Authorization: `Basic ${basicAuth}` },
      multipart: {
        file: {
          name: `${processKey}.bpmn20.xml`,
          mimeType: 'application/xml',
          buffer: Buffer.from(bpmn(processKey, processName, taskName, formKey)),
        },
      },
    });
    expect(processDeploy.status()).toBe(201);

    // --- Login and land on the task app ---
    await login(page, ADMIN_USER, ADMIN_PASSWORD);
    await page.goto('/');
    await expect(page.locator('.apps-wrapper')).toBeAttached();

    // --- Start the process from the workflow UI ---
    await page.goto('/workflow/#/start-process');
    await expect(page.getByRole('heading', { name: 'Start Workflow' })).toBeVisible();
    await page.locator('ul.simple-list.selection li', { hasText: processName }).click();
    await page.getByRole('button', { name: 'Start process' }).click();

    // --- Wait for the engine to actually hold the task before trusting any
    //     UI list: the Angular list caches, and under full parallelism other
    //     specs' instances make a premature read flaky. ---
    await expect
      .poll(
        async () => {
          const res = await page.request.post(`${BASE_URL}/app/rest/query/tasks`, {
            data: { text: taskName, state: 'open', size: 5 },
          });
          if (res.status() !== 200) return 0;
          const body = (await res.json()) as { data: Array<{ id: string }> };
          return body.data.length;
        },
        { timeout: 15_000 },
      )
      .toBeGreaterThan(0);

    // --- Open the task from the task list. Force a full reload first:
    //     hash-only goto inside the Angular app does not re-route reliably,
    //     and the process-detail auto-selection is racy when other specs
    //     start instances at the same time. ---
    await page.goto('about:blank');
    await page.goto('/workflow/#/tasks');
    const listedTask = page.locator('ul.full-list li', { hasText: taskName });
    await expect(listedTask).toBeVisible();
    await listedTask.click();
    await expect(page.locator('.main-content-wrapper h2').first()).toHaveText(taskName);

    // --- The form renders against the deployed form key (REST read on the
    //     same session), then complete through the UI outcome button ---
    const query = await page.request.post(`${BASE_URL}/app/rest/query/tasks`, {
      data: { text: taskName, state: 'open', size: 5 },
    });
    expect(query.status()).toBe(200);
    const queryBody = (await query.json()) as { data: Array<{ id: string }> };
    const taskId = queryBody.data[0]?.id;
    expect(taskId, 'task id from the open-task query').toBeTruthy();
    const taskForm = await page.request.get(`${BASE_URL}/app/rest/task-forms/${taskId}`);
    expect(taskForm.status()).toBe(200);
    const formData = (await taskForm.json()) as { key?: string; id?: string };
    expect(formData.key ?? formData.id).toBe(formKey);

    const completeButton = page.locator('button#form_complete_button', { hasText: 'Complete' }).first();
    await expect(completeButton).toBeVisible();
    await completeButton.click();

    // --- The task is gone from the open list (force a full reload: hash-only
    //     goto inside the Angular app does not re-route reliably) ---
    await page.goto('about:blank');
    await page.goto('/workflow/#/tasks');
    // An empty open list renders the "no tasks" placeholder; the list itself
    // stays hidden, so only assert the completed task is absent.
    await expect(page.locator('ul.full-list li', { hasText: taskName })).toHaveCount(0);

    // --- REST backstop: the instance finished with its history recorded ---
    const historic = await engineGetJson<{ total: number; data: Array<{ endTime?: string }> }>(
      page,
      `/history/historic-process-instances?processDefinitionKey=${processKey}&finished=true`,
    );
    expect(historic.status).toBe(200);
    expect(historic.body.total).toBeGreaterThanOrEqual(1);
  });
});
