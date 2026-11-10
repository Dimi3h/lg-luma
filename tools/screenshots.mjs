// Capture documentation from the actual native desktop renderer, not a browser.
import fs from 'node:fs/promises';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const root = path.resolve(import.meta.dirname, '..');
const docs = path.join(root, 'docs');
const scratch = path.join(root, '.build');
await fs.mkdir(docs, { recursive: true });
await fs.mkdir(scratch, { recursive: true });

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', timeout: 120000 });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited with status ${result.status}`);
}

// Force the desktop feature set even after a previous all-features lint/build.
run('cargo', ['build', '--locked', '--package', 'luma', '--no-default-features']);
const binary = path.resolve(root, process.env.CARGO_TARGET_DIR ?? 'target', 'debug/luma');
const temporary = await fs.mkdtemp(path.join(scratch, 'documentation-'));
const preferences = path.join(temporary, 'preferences.json');
const configuration = {
  language: 'en',
  background: 'dunes',
  hour_24: false,
  show_weather: true,
  weather_location: null,
  hidden: [],
  input_modes: {},
  reduced_motion: false,
};
try {
  for (const [name, script] of [
    ['home', 'home'],
    // The grid below the third row, with the top shelf scrolled away.
    ['apps', 'home,down,down,down'],
    ['settings', 'settings'],
    // Widgets lists Date, Greeting, Weather, Shortcuts, then Inputs.
    ['inputs', 'settings,down,down,enter,down,down,down,down,enter'],
    // Up from the grid reaches the weather button; Right reaches the Apps and Control Center
    // icons in the top bar.
    ['control-center', 'home,up,right,right,enter'],
  ]) {
    // Independent state makes screenshots stable regardless of capture order.
    await fs.writeFile(preferences, JSON.stringify(configuration));
    run(binary, [
      '--assets',
      path.join(scratch, 'assets'),
      '--preferences',
      preferences,
      '--time',
      '2026-10-22T20:41:00+02:00',
      '--script',
      script,
      '--frames',
      '90',
      '--capture-scale',
      '1',
      '--screenshot',
      path.join(docs, `${name}.png`),
    ]);
    console.log(`Native screenshot: docs/${name}.png (1920×1080)`);
  }
} finally {
  await fs.rm(temporary, { recursive: true, force: true });
}
