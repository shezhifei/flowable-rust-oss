import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: 'list',
  use: {
    baseURL: 'http://127.0.0.1:4174/modeler-app/',
    trace: 'on-first-retry',
  },
  webServer: {
    command: 'vite preview --host 127.0.0.1',
    url: 'http://127.0.0.1:4174/modeler-app/',
    timeout: 20_000,
    reuseExistingServer: !process.env.CI,
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
