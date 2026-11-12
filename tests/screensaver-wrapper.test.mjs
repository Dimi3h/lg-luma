import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

const source = await fs.readFile(new URL('../tv/screensaver.sh', import.meta.url), 'utf8');

// tv/screensaver.sh in a temporary root: the copied state directory with a recording
// luma-helper, the cold-boot hook as a real symlink, and stubs for the root-only id/stat checks.
async function fixture({ helper = true, uid = 0 } = {}) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'luma-screensaver-wrapper-'));
  const state = path.join(root, 'state');
  const bin = path.join(root, 'bin');
  const hooks = path.join(root, 'init.d');
  const calls = path.join(root, 'calls');
  await Promise.all([state, bin, hooks].map((dir) => fs.mkdir(dir)));
  const line = 'STATE=/var/lib/luma-screensaver\n';
  assert.ok(source.includes(line));
  await fs.writeFile(
    path.join(state, 'screensaver.sh'),
    source.replace(line, `STATE='${state}'\n`),
    { mode: 0o755 },
  );
  if (helper) {
    await fs.writeFile(
      path.join(state, 'luma-helper'),
      `#!/bin/sh\nfor a; do printf '[%s]' "$a"; done >>'${calls}'\necho >>'${calls}'\n`,
      { mode: 0o755 },
    );
  }
  for (const [name, body] of [
    ['id', `echo ${uid}`],
    ['stat', 'echo 0'],
    // macOS has no flock(1); the lock itself is not under test here.
    ['flock', 'exit 0'],
  ]) {
    await fs.writeFile(path.join(bin, name), `#!/bin/sh\n${body}\n`, { mode: 0o755 });
  }
  const hook = path.join(hooks, '10-luma-screensaver');
  await fs.symlink(path.join(state, 'screensaver.sh'), hook);
  const run = (args, input = '') =>
    spawnSync('/bin/sh', [hook, ...args], {
      encoding: 'utf8',
      input,
      env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
      timeout: 10000,
    });
  const recorded = async (wait = true) => {
    for (let i = 0; i < (wait ? 200 : 1); i++) {
      const text = await fs.readFile(calls, 'utf8').catch(() => '');
      if (text) return text.trim().split('\n');
      if (wait) await new Promise((resolve) => setTimeout(resolve, 50));
    }
    return [];
  };
  const done = () => fs.rm(root, { recursive: true, force: true });
  return { root, state, run, recorded, done };
}

test('status through the boot-hook symlink runs the copied helper', async () => {
  const tv = await fixture();
  try {
    const status = tv.run(['status']);
    assert.equal(status.status, 0, status.stderr);
    assert.deepEqual(await tv.recorded(), [`[screensaver][status][${tv.state}]`]);
  } finally {
    await tv.done();
  }
});

test('a cold boot schedules the copied apply at once and logs it', async () => {
  const tv = await fixture();
  try {
    const boot = tv.run([]);
    assert.equal(boot.status, 0, boot.stderr);
    assert.deepEqual(await tv.recorded(), [`[screensaver][apply][${tv.state}]`]);
    const log = await fs.readFile(path.join(tv.state, 'boot.log'), 'utf8');
    assert.match(log, /^Luma screensaver boot hook: \d{4}-\d\d-\d\d \d\d:\d\d:\d\d, uptime /m);
  } finally {
    await tv.done();
  }
});

test('enable explains every change and asks first; only yes reaches the helper', async () => {
  const tv = await fixture();
  try {
    const declined = tv.run(['enable', '/apps/com.ecklf.luma'], 'maybe\nn\n');
    assert.equal(declined.status, 0, declined.stderr);
    assert.match(declined.stdout, /Bind-mount .*com\.webos\.app\.screensaver/);
    assert.match(declined.stdout, /10-luma-screensaver/);
    assert.match(declined.stdout, /Restore LG's screensaver with: sh .*screensaver\.sh disable/);
    assert.match(declined.stdout, /Please answer y or n\./);
    assert.match(declined.stdout, /Cancelled\. No screensaver changes applied\./);
    assert.deepEqual(await tv.recorded(false), []);
    const closed = tv.run(['enable', '/apps/com.ecklf.luma'], '');
    assert.equal(closed.status, 1);
    assert.deepEqual(await tv.recorded(false), []);
    const accepted = tv.run(['enable', '/apps/com.ecklf.luma'], 'y\n');
    assert.equal(accepted.status, 0, accepted.stderr);
    assert.deepEqual(await tv.recorded(), ['[screensaver][enable][/apps/com.ecklf.luma]']);
  } finally {
    await tv.done();
  }
});

test('disable needs no confirmation and restores through the helper', async () => {
  const tv = await fixture();
  try {
    const disable = tv.run(['disable']);
    assert.equal(disable.status, 0, disable.stderr);
    assert.deepEqual(await tv.recorded(), [`[screensaver][disable][${tv.state}]`]);
  } finally {
    await tv.done();
  }
});

test('unknown commands, non-root users and a missing helper change nothing', async () => {
  for (const [options, args, message] of [
    [{}, ['remove'], /Usage: screensaver\.sh enable/],
    [{ uid: 501 }, ['status'], /Run as root\./],
    [{ helper: false }, ['status'], /native Luma TV helper is missing/],
  ]) {
    const tv = await fixture(options);
    try {
      const result = tv.run(args);
      assert.equal(result.status, 1);
      assert.match(result.stderr, message);
      assert.deepEqual(await tv.recorded(false), []);
    } finally {
      await tv.done();
    }
  }
});
