import { expect, test } from '@playwright/test';
import { ADMIN_PASSWORD, ADMIN_USER, BASE_URL, login } from '../helpers/server';

/**
 * L4 identity: create a user through the idm app UI, grant it the task-app
 * privilege through the idm REST aggregation on the admin session, then prove
 * the new identity can use the workflow app but is refused by the admin app.
 */
test.describe('L4 identity', () => {
  test('create user, grant access-task, new identity is scoped correctly', async ({ page }) => {
    const suffix = Math.random().toString(36).slice(2, 8);
    const userId = `l4-user-${suffix}`;
    const password = `l4-pass-${suffix}`;

    // --- Create the user through the idm UI on the admin session ---
    await login(page, ADMIN_USER, ADMIN_PASSWORD);
    await page.goto('/idm/#/user-mgmt');
    await page.getByRole('button', { name: 'Create user' }).click();

    const modal = page.locator('.modal-content');
    await expect(modal).toBeVisible();
    await modal.locator('input[name="idInput"]').fill(userId);
    await modal.locator('input[name="emailInput"]').fill(`${userId}@example.com`);
    await modal.locator('input[type="password"]').fill(password);
    await modal.locator('input[ng-model="model.user.firstName"]').fill('L4');
    await modal.locator('input[ng-model="model.user.lastName"]').fill(`User ${suffix}`);
    await modal.locator('.modal-footer button.btn-primary').click();

    // The modal hides and the reloaded list shows the new user.
    await expect(modal).toBeHidden();
    await expect(page.locator('table.users tr', { hasText: userId })).toBeVisible();

    // --- Grant access-task through the idm REST aggregation (same cookie) ---
    const privileges = await page.request.get(`${BASE_URL}/idm-app/rest/admin/privileges`);
    expect(privileges.status()).toBe(200);
    const accessTask = ((await privileges.json()) as Array<{ id: string; name: string }>).find(
      (privilege) => privilege.name === 'access-task',
    );
    expect(accessTask, 'access-task privilege seeded by global setup').toBeTruthy();
    const grant = await page.request.post(
      `${BASE_URL}/idm-app/rest/admin/privileges/${accessTask!.id}/users`,
      { data: { userId } },
    );
    expect(grant.status(), JSON.stringify(await grant.text())).toBe(200);

    // --- The new identity can use the workflow app ---
    await login(page, userId, password);
    const account = await page.request.get(`${BASE_URL}/app/rest/account`);
    expect(account.status()).toBe(200);
    expect(((await account.json()) as { id: string }).id).toBe(userId);

    const tasks = await page.request.post(`${BASE_URL}/app/rest/query/tasks`, {
      data: { state: 'open', size: 5 },
    });
    expect(tasks.status()).toBe(200);

    // --- ...but is refused by the admin app (no access-admin privilege) ---
    const adminRest = await page.request.get(`${BASE_URL}/admin-app/rest/server-configs`);
    expect(adminRest.status()).toBe(403);
    const adminPage = await page.request.get(`${BASE_URL}/admin/`);
    expect([401, 403]).toContain(adminPage.status());
  });
});
