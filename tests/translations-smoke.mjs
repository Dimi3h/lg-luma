import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import http from 'node:http';
import { spawn } from 'node:child_process';
import messages from '../assets/messages.json' with { type: 'json' };

// Desktop runs stay hidden on macOS so they never take the foreground (see --headless).
if (process.platform === 'darwin') process.env.LUMA_HEADLESS ??= '1';

const root = path.resolve(import.meta.dirname, '..');
const out = path.join(root, '.build');
await fs.mkdir(out, { recursive: true });
const temporary = await fs.mkdtemp(path.join(out, 'translations-smoke-'));
const preferences = path.join(temporary, 'preferences.json');
const binary = path.resolve(root, process.env.CARGO_TARGET_DIR ?? 'target', 'debug/luma');
const requests = [];
const places = {
  en: { name: 'Vienna', region: 'Vienna', country: 'Austria' },
  nl: { name: 'Wenen', region: 'Wenen', country: 'Oostenrijk' },
};
const server = http.createServer((request, response) => {
  const url = new URL(request.url, 'http://127.0.0.1');
  requests.push({ path: url.pathname, query: Object.fromEntries(url.searchParams) });
  response.setHeader('Content-Type', 'application/json');
  if (url.pathname === '/v1/search') {
    const place = places[url.searchParams.get('language')];
    assert.ok(place, 'Geocoding must use a supported UI language');
    response.end(
      JSON.stringify({
        results: [{ ...place, admin1: place.region, latitude: 48.2085, longitude: 16.3721 }],
      }),
    );
  } else if (url.pathname === '/v1/forecast') {
    assert.equal(url.searchParams.get('temperature_unit'), 'celsius');
    response.end(JSON.stringify({ current: { temperature_2m: 14.2, weather_code: 2, is_day: 1 } }));
  } else {
    response.writeHead(404);
    response.end('{}');
  }
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
async function run(command, args) {
  await new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: root, stdio: 'inherit' });
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      reject(new Error('Translation smoke child timed out'));
    }, 45000);
    child.on('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on('exit', (code) => {
      clearTimeout(timer);
      if (code === 0) resolve();
      else reject(new Error(`${command} exited ${code}`));
    });
  });
}
async function scenario(name, script, extra = [], frames = 120) {
  const dump = path.join(temporary, `${name}.json`);
  const report = path.join(out, `language-${name}-render.json`);
  await run(binary, [
    '--preferences',
    preferences,
    '--weather-test-server',
    origin,
    '--script',
    script,
    '--frames',
    String(frames),
    '--time',
    '2026-10-21T20:41:00+02:00',
    '--screenshot',
    path.join(out, `language-${name}.png`),
    '--dump-state',
    dump,
    '--diagnostics',
    report,
    ...extra,
  ]);
  const state = JSON.parse(await fs.readFile(dump, 'utf8'));
  const render = JSON.parse(await fs.readFile(report, 'utf8'));
  assert.equal(state.error, null);
  assert.equal(
    render.capture.missing_glyphs,
    0,
    'Translated text must not become missing-character boxes',
  );
  assert.equal(render.capture.glyph_failures, 0);
  assert.equal(
    render.capture.missing_images,
    0,
    'Settings icons and other translated-screen artwork must be present',
  );
  return state;
}
function contains(state, key, language) {
  assert.ok(state.ui_text.includes(messages[key][language]), `Missing rendered ${key}/${language}`);
}
const picker = 'settings,enter,down,down,enter';
const weatherPage = 'settings,down,down,enter,down,down,enter';
const inputsPage = 'settings,down,down,enter,down,down,down,down,enter';
const location = `${weatherPage},down,enter`;
try {
  if (process.env.LUMA_SMOKE_SKIP_BUILD !== '1')
    await run('cargo', ['build', '--locked', '--package', 'luma']);
  await fs.writeFile(
    preferences,
    JSON.stringify({ language: 'en', show_app_names: true, reduced_motion: true }),
  );
  const languages = await scenario('picker-en', picker);
  assert.equal(languages.subpage, 'Language');
  assert.ok(languages.ui_text.includes('English'));
  assert.ok(languages.ui_text.includes('Nederlands'));
  for (const removed of ['Deutsch', '简体中文', 'Español', 'Tiếng Việt'])
    assert.ok(!languages.ui_text.includes(removed), `${removed} is no longer offered`);

  const dutch = await scenario('nl-home', `${picker},down,enter,home`);
  assert.equal(dutch.preferences.language, 'nl');
  for (const key of ['evening', 'setWeatherLocation']) contains(dutch, key, 'nl');
  // After the clock, the Dutch date keeps Dutch lowercase.
  assert.ok(dutch.ui_text.includes('woensdag 21 oktober'));
  assert.ok(dutch.ui_text.includes('20:41'), 'Dutch defaults to a 24-hour clock');
  assert.ok(dutch.ui_text.includes('Netflix'), 'Installed app names stay intact');
  const persisted = await scenario('nl-settings', 'settings');
  assert.equal(persisted.preferences.language, 'nl', 'Dutch is restored after restart');
  assert.ok(persisted.ui_text.includes('Nederlands'));
  for (const key of ['language', 'appNames', 'clock', 'settings']) contains(persisted, key, 'nl');

  const selected = await scenario(
    'nl-weather',
    `${location},type:Wenen,back,enter,await-search,down,enter,home,await-weather`,
  );
  assert.equal(selected.preferences.weather_location.name, 'Wenen');
  assert.equal(selected.preferences.weather_location.country, 'Oostenrijk');
  contains(selected, 'partlyCloudy', 'nl');
  assert.equal(
    requests.filter((request) => request.path === '/v1/search').at(-1).query.language,
    'nl',
    'Geocoding must use the Dutch locale',
  );
  const forecasts = requests.filter((request) => request.path === '/v1/forecast').length;
  const changed = await scenario('en-weather', `${picker},up,enter,home,await-weather`);
  assert.equal(changed.preferences.language, 'en');
  contains(changed, 'partlyCloudy', 'en');
  assert.ok(changed.ui_text.includes('Wenen'), 'The saved city name is data, not a UI label');
  assert.equal(
    requests.filter((request) => request.path === '/v1/forecast').length,
    forecasts,
    'Language changes reuse weather cache',
  );

  const saved = JSON.parse(await fs.readFile(preferences, 'utf8'));
  await fs.writeFile(preferences, JSON.stringify({ ...saved, language: 'nl' }));
  const dialog = await scenario('nl-dialog', 'home,menu');
  for (const key of ['open', 'removeFavorite', 'hideApp', 'deleteApp']) contains(dialog, key, 'nl');
  const deleteConfirmation = 'home,menu,down,down,down,enter';
  const deletion = await scenario('nl-delete-confirm', deleteConfirmation);
  assert.equal(deletion.focus, 'CancelDelete');
  contains(deletion, 'cancel', 'nl');
  assert.ok(
    deletion.ui_text.includes(messages.deleteAppWarning.nl.replace('{app}', 'Netflix')),
    'The deletion warning must be Dutch and preserve the installed app name',
  );
  const cancelled = await scenario('nl-delete-cancel', `${deleteConfirmation},enter`);
  assert.ok(cancelled.apps.includes('netflix'), 'Cancel must not delete the app');
  const input = await scenario('nl-input-options', `${inputsPage},down,enter`);
  for (const key of ['stateOn', 'stateOff', 'stateAuto', 'inputWhenConnected'])
    contains(input, key, 'nl');
  const background = await scenario('nl-background', 'settings,down,enter');
  for (const key of ['background', 'uploadWallpaper']) contains(background, key, 'nl');
  const backgroundOptions = await scenario(
    'nl-background-options',
    'settings,down,enter,down,enter',
  );
  assert.equal(backgroundOptions.modal, true);
  for (const key of ['useBackground', 'deleteBackground']) contains(backgroundOptions, key, 'nl');
  // The linear keyboard's row ends with Space, Delete, Clear and Done; move there first.
  const keyboard = await scenario(
    'nl-keyboard',
    `${location},enter,${Array(39).fill('right').join(',')}`,
    [],
    320,
  );
  for (const key of ['space', 'delete', 'clear', 'done']) contains(keyboard, key, 'nl');
  const weatherSettings = await scenario('nl-weather-settings', weatherPage);
  for (const key of ['widgets', 'showWidget', 'location', 'temperature'])
    contains(weatherSettings, key, 'nl');
  assert.ok(
    !weatherSettings.ui_text.some((label) => label.includes('Open-Meteo')),
    'Weather credit is not shown in the UI',
  );
  await scenario('nl-4k', 'home', ['--capture-scale', '2']);

  // A language earlier versions offered falls back to English instead of failing to load.
  await fs.writeFile(
    preferences,
    JSON.stringify({ language: 'de', show_app_names: true, reduced_motion: true }),
  );
  const fallback = await scenario('removed-language', 'home');
  assert.equal(fallback.preferences.language, 'en');
  contains(fallback, 'evening', 'en');
  console.log(
    'Native translation checks passed: English and Dutch only, saved language selection and restart, Dutch dates, localized geocoding/weather with cache reuse, dialogs, keyboard and settings labels, unchanged app names, removed-language fallback and 4K rendering.',
  );
} finally {
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
  await fs.rm(temporary, { recursive: true, force: true });
}
