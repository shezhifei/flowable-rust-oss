import { expect, test } from '@playwright/test';

import { ADMIN_PASSWORD, ADMIN_USER, BASE_URL, login } from '../helpers/server';

/**
 * Harness smoke: the server is up with enforced auth, the static bundles are
 * served, and a real cookie login works. Not one of the four V0 links; this
 * only guards the fixture itself.
 */
test.describe('harness smoke', () => {
  test('health, static bundles and cookie login', async ({ page }) => {
    const health = await page.request.get(`${BASE_URL}/health`);
    expect(health.ok()).toBeTruthy();

    // The login app is public (Java permitAll); /admin/ sits behind the
    // access-admin privilege in the Java authorize-requests table, so an
    // anonymous request is rejected rather than served.
    const idm = await page.request.get(`${BASE_URL}/idm/`);
    expect(idm.status()).toBe(200);
    const adminAnonymous = await page.request.get(`${BASE_URL}/admin/`);
    expect(adminAnonymous.status()).toBe(401);

    // Engine REST rejects anonymous callers in enforced mode.
    const anonymous = await page.request.get(`${BASE_URL}/repository/models`);
    expect(anonymous.status()).toBe(401);

    await login(page, ADMIN_USER, ADMIN_PASSWORD);

    // The cookie session reaches the engine API (stream D SSO fix) and the
    // privilege-gated modeler surface.
    const models = await page.request.get(`${BASE_URL}/repository/models`);
    expect(models.status()).toBe(200);
    const modeler = await page.request.get(`${BASE_URL}/modeler-app/`);
    expect(modeler.status()).toBe(200);
  });
});
