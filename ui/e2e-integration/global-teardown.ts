/**
 * Stops the server started by global-setup and removes its isolated
 * database/log directory. The state file is the only hand-off between the two
 * hook processes.
 */
import { existsSync, readFileSync, rmSync, unlinkSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const STATE_FILE = join(dirname(fileURLToPath(import.meta.url)), '.server-state.json');

interface ServerState {
  pid: number;
  workDir: string;
}

function killProcess(pid: number): void {
  try {
    process.kill(pid);
  } catch {
    // Already gone.
  }
  if (process.platform === 'win32') {
    try {
      execFileSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' });
    } catch {
      // Already gone.
    }
  }
}

export default async function globalTeardown(): Promise<void> {
  if (!existsSync(STATE_FILE)) return;
  const state = JSON.parse(readFileSync(STATE_FILE, 'utf8')) as ServerState;

  killProcess(state.pid);

  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    try {
      process.kill(state.pid, 0);
      await new Promise((resolve) => setTimeout(resolve, 200));
    } catch {
      break;
    }
  }

  rmSync(state.workDir, { recursive: true, force: true });
  unlinkSync(STATE_FILE);
  console.log(`[global-teardown] stopped pid=${state.pid}, removed ${state.workDir}`);
}
