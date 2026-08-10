/**
 * Assembles the static assets the integration server serves:
 *
 * - `ui/legacy/{idm,admin,task}` — the three AngularJS bundles, copied into the
 *   repo by stream A and served via FLOWABLE_UI_STATIC_DIR.
 * - `ui/modeler/dist` — the first-party modeler production bundle, served via
 *   FLOWABLE_MODELER_STATIC_DIR. Built here when missing.
 *
 * The server mounts the two roots itself; this script only guarantees both
 * exist on disk.
 */
import { existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');

const legacyRoot = join(repoRoot, 'ui', 'legacy');
for (const app of ['idm', 'admin', 'task']) {
  if (!existsSync(join(legacyRoot, app))) {
    throw new Error(`Legacy bundle missing: ${join(legacyRoot, app)}`);
  }
}

const modelerDist = join(repoRoot, 'ui', 'modeler', 'dist');
if (!existsSync(join(modelerDist, 'index.html'))) {
  console.log('[assets] ui/modeler/dist missing; building the modeler bundle');
  const result = spawnSync('npm', ['run', 'build'], {
    cwd: join(repoRoot, 'ui', 'modeler'),
    stdio: 'inherit',
    shell: process.platform === 'win32',
  });
  if (result.status !== 0 || !existsSync(join(modelerDist, 'index.html'))) {
    throw new Error('Modeler build failed; cannot assemble integration assets');
  }
}

console.log(`[assets] legacy root: ${legacyRoot}`);
console.log(`[assets] modeler dist: ${modelerDist}`);
