/**
 * Shared constants and helpers for the integration specs. Everything talks to
 * the real server started by global-setup; no mocks anywhere.
 */
import type { Page } from '@playwright/test';

export const BASE_URL = 'http://127.0.0.1:8085';

/** Seeded by the platform bootstrap (env in global-setup) and granted every
 *  `access-*` privilege by the seeding step there. */
export const ADMIN_USER = 'admin';
export const ADMIN_PASSWORD = 'e2e-admin-password';

export const ALL_PRIVILEGES = [
  'access-idm',
  'access-admin',
  'access-task',
  'access-modeler',
  'access-rest-api',
] as const;

/** Cookie login through the real endpoint; the session cookie lands in the
 *  page's browser context, so subsequent same-origin UI and XHR calls carry
 *  it. */
export async function login(page: Page, user: string, password: string): Promise<void> {
  const response = await page.request.post(`${BASE_URL}/app/authentication`, {
    form: { j_username: user, j_password: password },
  });
  if (!response.ok()) {
    throw new Error(`Login failed for '${user}': ${response.status()} ${await response.text()}`);
  }
}

/** Engine REST call with HTTP Basic (the API surface's own scheme). */
export async function engineGet(
  page: Page,
  path: string,
  user = ADMIN_USER,
  password = ADMIN_PASSWORD,
): Promise<Response> {
  const auth = Buffer.from(`${user}:${password}`).toString('base64');
  const response = await page.request.get(`${BASE_URL}${path}`, {
    headers: { Authorization: `Basic ${auth}` },
  });
  return response as unknown as Response;
}

export async function engineGetJson<T = unknown>(
  page: Page,
  path: string,
  user = ADMIN_USER,
  password = ADMIN_PASSWORD,
): Promise<{ status: number; body: T }> {
  const auth = Buffer.from(`${user}:${password}`).toString('base64');
  const response = await page.request.get(`${BASE_URL}${path}`, {
    headers: { Authorization: `Basic ${auth}` },
  });
  return { status: response.status(), body: (await response.json()) as T };
}

export async function enginePostJson<T = unknown>(
  page: Page,
  path: string,
  payload: unknown,
  user = ADMIN_USER,
  password = ADMIN_PASSWORD,
): Promise<{ status: number; body: T }> {
  const auth = Buffer.from(`${user}:${password}`).toString('base64');
  const response = await page.request.post(`${BASE_URL}${path}`, {
    headers: { Authorization: `Basic ${auth}` },
    data: payload,
  });
  const text = await response.text();
  let body: unknown = null;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {
    body = text;
  }
  return { status: response.status(), body: body as T };
}
