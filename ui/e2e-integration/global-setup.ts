/**
 * Bootstraps the real stack once per run:
 *
 * 1. `npm run assets` — guarantees ui/legacy and ui/modeler/dist exist.
 * 2. `cargo build -p flowable-rest` — incremental; a no-op when the debug
 *    binary already matches the sources.
 * 3. Spawn the binary with an isolated SQLite file database, the enforced
 *    auth defaults, a seeded bootstrap admin, and the admin server-config
 *    store pointing at itself.
 * 4. Poll /health until the server reports UP (no fixed sleeps).
 * 5. Seed the five `access-*` privileges and grant them to the admin user
 *    through the engine REST API (Basic), mirroring what the Java IDM
 *    bootstrap does on first start.
 *
 * State for the teardown (pid, temp dir) is written to .server-state.json.
 */
import { spawn, spawnSync } from 'node:child_process';
import { createWriteStream, existsSync, mkdtempSync, writeFileSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..', '..');
const HOST = '127.0.0.1';
const PORT = 8085;
const BASE_URL = `http://${HOST}:${PORT}`;
const ADMIN_USER = 'admin';
const ADMIN_PASSWORD = 'e2e-admin-password';
const STATE_FILE = join(here, '.server-state.json');
const HEALTH_TIMEOUT_MS = 90_000;
const HEALTH_INTERVAL_MS = 250;

const ALL_PRIVILEGES = [
  'access-idm',
  'access-admin',
  'access-task',
  'access-modeler',
  'access-rest-api',
];

function run(command: string, args: string[], cwd: string): void {
  const result = spawnSync(command, args, {
    cwd,
    stdio: 'inherit',
    shell: process.platform === 'win32',
  });
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed with exit code ${result.status}`);
  }
}

async function health(): Promise<boolean> {
  try {
    const response = await fetch(`${BASE_URL}/health`, {
      signal: AbortSignal.timeout(2_000),
    });
    return response.ok;
  } catch {
    return false;
  }
}

async function waitForServer(logTail: () => string): Promise<void> {
  const deadline = Date.now() + HEALTH_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (await health()) return;
    await new Promise((resolve) => setTimeout(resolve, HEALTH_INTERVAL_MS));
  }
  throw new Error(
    `flowable-rest did not report healthy within ${HEALTH_TIMEOUT_MS}ms.\n` +
      `Last server log lines:\n${logTail()}`,
  );
}

async function seedPrivileges(): Promise<void> {
  const auth = Buffer.from(`${ADMIN_USER}:${ADMIN_PASSWORD}`).toString('base64');
  const headers = {
    Authorization: `Basic ${auth}`,
    'Content-Type': 'application/json',
  };
  for (const privilege of ALL_PRIVILEGES) {
    const created = await fetch(`${BASE_URL}/identity/privileges`, {
      method: 'POST',
      headers,
      body: JSON.stringify({ id: privilege, name: privilege }),
    });
    if (!created.ok) {
      throw new Error(`Seeding privilege '${privilege}' failed: ${created.status}`);
    }
    const granted = await fetch(`${BASE_URL}/privileges/${privilege}/users`, {
      method: 'POST',
      headers,
      body: JSON.stringify({ userId: ADMIN_USER }),
    });
    if (!granted.ok) {
      throw new Error(`Granting privilege '${privilege}' failed: ${granted.status}`);
    }
  }
}

export default async function globalSetup(): Promise<void> {
  if (await health()) {
    throw new Error(
      `${BASE_URL} already answers /health. Another flowable-rest instance is ` +
        'running on the frozen integration port; stop it before running the e2e suite.',
    );
  }

  run('npm', ['run', 'assets'], here);
  run('cargo', ['build', '-p', 'flowable-rest'], repoRoot);

  const binary = join(
    repoRoot,
    'target',
    'debug',
    process.platform === 'win32' ? 'flowable-rest.exe' : 'flowable-rest',
  );
  if (!existsSync(binary)) {
    throw new Error(`flowable-rest binary not found at ${binary} after cargo build`);
  }

  const workDir = mkdtempSync(join(tmpdir(), 'flowable-e2e-'));
  const logPath = join(workDir, 'server.log');
  const logStream = createWriteStream(logPath);

  const child = spawn(binary, [], {
    cwd: workDir,
    env: {
      ...process.env,
      FLOWABLE_SERVER_BIND_ADDRESS: `${HOST}:${PORT}`,
      FLOWABLE_PROCESS_DATABASE_PATH: join(workDir, 'flowable-e2e.db'),
      FLOWABLE_BOOTSTRAP_CREATE_DEFAULT_ADMIN: 'true',
      FLOWABLE_BOOTSTRAP_ADMIN_USER_ID: ADMIN_USER,
      FLOWABLE_BOOTSTRAP_ADMIN_PASSWORD: ADMIN_PASSWORD,
      FLOWABLE_UI_STATIC_DIR: join(repoRoot, 'ui', 'legacy'),
      FLOWABLE_MODELER_STATIC_DIR: join(repoRoot, 'ui', 'modeler', 'dist'),
      FLOWABLE_UI_SERVER_CONFIG_PATH: join(workDir, 'server-configs.json'),
      FLOWABLE_UI_ENGINE_HOST: `http://${HOST}`,
      FLOWABLE_UI_ENGINE_PORT: String(PORT),
      FLOWABLE_UI_ENGINE_USER: ADMIN_USER,
      FLOWABLE_UI_ENGINE_PASSWORD: ADMIN_PASSWORD,
      // FLOWABLE_UI_AUTH_MODE deliberately unset: enforced is the default and
      // exactly what this suite exists to exercise.
      RUST_LOG: process.env.RUST_LOG ?? 'info',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  child.stdout?.pipe(logStream);
  child.stderr?.pipe(logStream);

  const logTail = () => {
    try {
      return readFileSync(logPath, 'utf8').split('\n').slice(-30).join('\n');
    } catch {
      return '(log unavailable)';
    }
  };

  try {
    await waitForServer(logTail);
    await seedPrivileges();
  } catch (error) {
    child.kill();
    throw error;
  }

  writeFileSync(
    STATE_FILE,
    JSON.stringify({ pid: child.pid, workDir, logPath }, null, 2),
  );
  console.log(`[global-setup] flowable-rest pid=${child.pid} db+logs in ${workDir}`);
}
