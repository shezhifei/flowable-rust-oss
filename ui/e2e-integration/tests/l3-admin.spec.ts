import { expect, test } from '@playwright/test';
import {
  ADMIN_PASSWORD,
  ADMIN_USER,
  BASE_URL,
  enginePostJson,
  login,
} from '../helpers/server';

function bpmn(processKey: string, processName: string, taskName: string): string {
  return `<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="http://flowable.org/bpmn">
  <process id="${processKey}" name="${processName}" isExecutable="true">
    <startEvent id="start"/>
    <sequenceFlow id="flow-start" sourceRef="start" targetRef="review"/>
    <userTask id="review" name="${taskName}" flowable:assignee="${ADMIN_USER}"/>
    <sequenceFlow id="flow-end" sourceRef="review" targetRef="end"/>
    <endEvent id="end"/>
  </process>
</definitions>`;
}

/**
 * L3 observation: deploy a process and start an instance through the engine
 * REST API (Basic) as setup, then observe both through the admin app — the
 * deployments grid and the process-instances grid must show the data, and the
 * admin REST aggregation behind them must answer on the session cookie.
 */
test.describe('L3 observation', () => {
  test('deployments and running instances are visible in the admin app', async ({ page }) => {
    const suffix = Math.random().toString(36).slice(2, 8);
    const processKey = `l3-process-${suffix}`;
    const processName = `L3 process ${suffix}`;
    const taskName = `L3 review ${suffix}`;

    // --- Setup: deploy the process and start one instance (engine REST) ---
    const basicAuth = Buffer.from(`${ADMIN_USER}:${ADMIN_PASSWORD}`).toString('base64');
    const deploy = await page.request.post(`${BASE_URL}/repository/deployments`, {
      headers: { Authorization: `Basic ${basicAuth}` },
      multipart: {
        file: {
          name: `${processKey}.bpmn20.xml`,
          mimeType: 'application/xml',
          buffer: Buffer.from(bpmn(processKey, processName, taskName)),
        },
      },
    });
    expect(deploy.status()).toBe(201);

    const started = await enginePostJson(page, '/runtime/process-instances', {
      processDefinitionKey: processKey,
    });
    // This engine REST implementation answers 200 with the instance body.
    expect(started.status, JSON.stringify(started.body)).toBe(200);
    const instanceId = (started.body as { id?: string }).id;
    expect(instanceId).toBeTruthy();

    // --- Login and observe the deployment in the admin app ---
    await login(page, ADMIN_USER, ADMIN_PASSWORD);
    await page.goto('/admin/#/deployments');
    const deploymentsGrid = page.locator('.grid-wrapper#deployments');
    await expect(
      deploymentsGrid.locator('.ngCellText', { hasText: processKey }).first(),
    ).toBeVisible();

    // --- Observe the running instance (force a full reload: hash-only goto
    //     inside the Angular app does not re-route reliably) ---
    await page.goto('about:blank');
    await page.goto('/admin/#/process-instances');
    const instancesGrid = page.locator('.grid-wrapper#process-instances');
    await expect(
      instancesGrid.locator('.ngCellText', { hasText: instanceId! }).first(),
    ).toBeVisible();

    // --- REST backstop on the same session cookie: the admin aggregation
    //     layer proxies both queries to the engine. ---
    const configs = await page.request.get(`${BASE_URL}/admin-app/rest/server-configs`);
    expect(configs.status()).toBe(200);
    expect(((await configs.json()) as unknown[]).length).toBeGreaterThanOrEqual(1);

    const deployments = await page.request.get(
      `${BASE_URL}/admin-app/rest/admin/deployments?nameLike=${processKey}`,
    );
    expect(deployments.status()).toBe(200);
    const deploymentsBody = (await deployments.json()) as { total: number };
    expect(deploymentsBody.total).toBeGreaterThanOrEqual(1);
  });
});
