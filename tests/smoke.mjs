import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import sharp from 'sharp';

// Desktop runs stay hidden on macOS so they never take the foreground (see --headless).
if (process.platform === 'darwin') process.env.LUMA_HEADLESS ??= '1';

const root = path.resolve(import.meta.dirname, '..');
const out = path.join(root, '.build');
await fs.mkdir(out, { recursive: true });
const run = (command, args) => {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', timeout: 120000 });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, `${command} failed`);
};
if (process.env.LUMA_SMOKE_SKIP_BUILD !== '1')
  run('cargo', ['build', '--locked', '--package', 'luma']);
const binary = path.resolve(root, process.env.CARGO_TARGET_DIR ?? 'target', 'debug/luma');
const temporary = await fs.mkdtemp(path.join(out, 'smoke-'));
try {
  const preferences = path.join(temporary, 'preferences.json');
  const scenario = async (name, script, extra = [], frames = 90) => {
    const screenshot = path.join(out, `${name}.png`);
    const dump = path.join(temporary, `${name}.json`);
    const diagnostics = path.join(out, `${name}-render.json`);
    run(binary, [
      '--preferences',
      preferences,
      '--script',
      script,
      '--frames',
      String(frames),
      '--screenshot',
      screenshot,
      '--dump-state',
      dump,
      '--diagnostics',
      diagnostics,
      ...extra,
    ]);
    const state = JSON.parse(await fs.readFile(dump, 'utf8'));
    const render = JSON.parse(await fs.readFile(diagnostics, 'utf8'));
    assert.equal(
      render.capture.glyph_failures,
      0,
      'Visible vector text must remain valid across cache reuse',
    );
    assert.equal(render.last_presented.glyph_failures, 0);
    assert.equal(render.capture.font_antialiasing, 'cpu glyph atlas, 256-level coverage');
    assert.equal(render.capture.pending_glyphs, 0, 'Captures wait for every glyph');
    assert.equal(render.capture.missing_glyphs, 0);
    assert.equal(render.capture.missing_images, 0);
    state.render = render;
    assert.equal(state.error, null);
    const stats = await sharp(screenshot).stats();
    assert.equal(
      stats.channels[3].min,
      255,
      'UI compositing must preserve opaque framebuffer alpha',
    );
    if (state.ui_text.includes('No recent apps')) {
      // The intentionally sparse empty switcher has no bright header/footer
      // controls. Verify its message pixels instead of full-screen variance.
      const messagePixels = await sharp(screenshot)
        .extract({ left: 360, top: 526, width: 1200, height: 60 })
        .toBuffer();
      const message = await sharp(messagePixels).stats();
      assert.ok(
        message.channels.slice(0, 3).some((channel) => channel.max > 200 && channel.stdev > 10),
      );
    } else {
      assert.ok(
        stats.channels.slice(0, 3).some((channel) => channel.stdev > 10),
        'Framebuffer must contain rendered UI',
      );
    }
    return state;
  };
  const home = await scenario('home', 'home');
  assert.deepEqual(home.focus, { App: 'netflix' });
  const recents = await scenario('recents', 'recents,await-recents');
  assert.equal(recents.modal, true);
  assert.deepEqual(recents.focus, { Recent: 'netflix' });
  assert.equal(recents.recents.length, 8);
  assert.equal(recents.recents_error, null);
  assert.equal(recents.recents_visibility, 1);
  assert.ok(!recents.ui_text.includes('Recent apps'));
  assert.ok(!recents.ui_text.includes('Pick up where you left off.'));
  assert.ok(!recents.ui_text.includes('Desktop preview · Sample history'));
  // As in the tvOS app switcher, only the focused card is titled.
  assert.ok(recents.ui_text.includes('Netflix'));
  assert.ok(!recents.ui_text.includes('YouTube'));
  assert.ok(!recents.ui_text.includes('1 / 8'));
  assert.ok(!recents.ui_text.includes('Close all'));
  assert.ok(
    !recents.ui_text.some((value) => value.includes('Switch') && value.includes('Close app')),
  );
  const recentClosed = await scenario(
    'recent-closed',
    'recents,await-recents,right,right,up,await-recents',
  );
  assert.equal(recentClosed.recents.length, 7);
  assert.ok(!recentClosed.recents.some((entry) => entry.app.id === 'com.apple.appletv'));
  assert.deepEqual(recentClosed.focus, { Recent: 'com.disney.disneyplus-prod' });
  const recentsEmpty = await scenario(
    'recents-empty',
    `recents,await-recents,${Array(8).fill('up,await-recents').join(',')}`,
    [],
    // Let the final card finish its upward exit before capturing the empty state.
    150,
  );
  assert.deepEqual(recentsEmpty.recents, []);
  assert.equal(recentsEmpty.focus, 'RecentEmpty');
  assert.ok(recentsEmpty.ui_text.includes('No recent apps'));
  assert.ok(!recentsEmpty.ui_text.includes('Apps you open will appear here.'));
  const recentBack = await scenario('recents-back', 'settings,recents,await-recents,back');
  assert.equal(recentBack.modal, false);
  assert.equal(recentBack.page, 'Settings');
  assert.equal(recentBack.recents_visibility, 0);
  const recentLaunch = await scenario(
    'recents-launch',
    'recents,await-recents,right,enter,await-recents',
  );
  assert.equal(recentLaunch.modal, false);
  const recentPointerLaunch = await scenario(
    'recents-pointer-launch',
    'recents,await-recents,click:960:527,await-recents',
  );
  assert.equal(recentPointerLaunch.modal, false);
  assert.equal(recentPointerLaunch.recents_visibility, 0);
  const recentHomeBack = await scenario('recents-home-back', 'home,back,await-recents');
  assert.equal(recentHomeBack.modal, true);
  assert.deepEqual(recentHomeBack.focus, { Recent: 'netflix' });
  const recentYoutube = await scenario('recents-youtube', 'recents,await-recents,right');
  assert.deepEqual(recentYoutube.focus, { Recent: 'youtube.leanback.v4' });
  // Move is the menu's first item and focused; Open, Hide and Delete follow.
  const appOptions = await scenario('app-options', 'home,menu');
  assert.deepEqual(appOptions.focus, { Option: 0 });
  assert.ok(appOptions.ui_text.includes('Move'));
  assert.ok(appOptions.ui_text.includes('Delete app'));
  const deleteFocused = await scenario('delete-focused', 'home,menu,down,down,down');
  assert.deepEqual(deleteFocused.focus, { Option: 2 });
  const deleteConfirm = await scenario('delete-confirm', 'home,menu,down,down,down,enter');
  assert.deepEqual(deleteConfirm.focus, 'CancelDelete');
  assert.ok(deleteConfirm.ui_text.some((value) => value.includes('uninstall “Netflix”')));
  const deleteCancel = await scenario('delete-cancel', 'home,menu,down,down,down,enter,enter');
  assert.deepEqual(deleteCancel.focus, { Option: 2 });
  assert.ok(deleteCancel.apps.includes('netflix'));
  const hidden = await scenario('apps', 'home,menu,down,down,enter');
  assert.ok(hidden.preferences.hidden.includes('netflix'));
  assert.deepEqual(hidden.focus, { App: 'youtube.leanback.v4' });
  assert.ok(!hidden.apps.includes('netflix'), 'Hidden apps leave the grid');
  // Reload from disk and show the hidden Netflix again from Settings › General › Hidden apps.
  const hiddenAppsPage = 'settings,enter,down,down,down,down,enter';
  const hiddenList = await scenario('hidden-apps', hiddenAppsPage);
  assert.equal(hiddenList.subpage, 'HiddenApps');
  assert.deepEqual(hiddenList.focus, { Choice: 0 });
  assert.ok(hiddenList.ui_text.includes('Netflix'));
  const restored = await scenario('restored', `${hiddenAppsPage},enter`);
  assert.deepEqual(restored.preferences.hidden, []);
  assert.ok(restored.ui_text.includes('No hidden apps.'), 'The empty state follows');
  const noSearch = await scenario('no-search', 'home,type:YouTube');
  assert.ok(!noSearch.ui_text.includes('Search'), 'There is no Search card');
  assert.equal(noSearch.modal, false, 'Typing on the main screen opens no keyboard');
  assert.ok(noSearch.apps.includes('netflix') && noSearch.apps.length > 1);
  for (const id of [
    'coupangplay',
    'com.frograms.watchaplay.webos',
    'pooq',
    'com.cmb.rainbowtv',
    'com.lgtvod.kr.app',
  ])
    assert.ok(noSearch.apps.includes(id), `${id} is in the grid`);
  const settings = await scenario('settings', 'settings,down,enter,down,enter,enter');
  assert.equal(settings.preferences.background, 'forest');
  // Hold OK, choose Move, move right, Back keeps the order (saved with the rest).
  const saved = await scenario('saved', 'home,hold,enter,right,back');
  assert.equal(saved.preferences.background, 'forest');
  assert.deepEqual(saved.preferences.app_order.slice(0, 2), ['youtube.leanback.v4', 'netflix']);
  const upload = await scenario(
    'wallpaper-upload',
    'settings,down,enter,up,up,up,enter,await-upload',
  );
  assert.equal(upload.modal, true);
  assert.equal(upload.focus, 'CloseDialog');
  assert.ok(upload.ui_text.includes('Upload'));
  assert.ok(upload.ui_text.includes('Scan this QR code'));
  assert.ok(upload.ui_text.includes('or enter this URL in the browser'));
  assert.ok(upload.ui_text.some((text) => text.startsWith('Keep this window open')));
  assert.ok(
    upload.ui_text.some((text) =>
      /^http:\/\/127\.0\.0\.1:\d+\/[1-9A-HJ-NP-Za-km-z]{8}$/.test(text),
    ),
  );
  const uploadClosed = await scenario(
    'wallpaper-upload-closed',
    'settings,down,enter,up,up,up,enter,await-upload,back',
  );
  assert.equal(uploadClosed.modal, false);
  assert.equal(uploadClosed.focus, 'UploadWallpaper');
  const customId = `custom-${'1'.repeat(32)}`;
  const customDirectory = path.join(temporary, 'wallpapers', customId);
  await fs.mkdir(customDirectory, { recursive: true });
  await sharp({ create: { width: 1920, height: 1080, channels: 3, background: '#263832' } })
    .jpeg()
    .toFile(path.join(customDirectory, 'wallpaper.jpg'));
  await sharp({ create: { width: 480, height: 270, channels: 3, background: '#749788' } })
    .jpeg()
    .toFile(path.join(customDirectory, 'thumbnail.jpg'));
  await fs.writeFile(path.join(customDirectory, 'metadata.json'), JSON.stringify({ id: customId }));
  const custom = await scenario('wallpaper-custom', 'settings,down,enter,down,enter,enter');
  assert.equal(custom.preferences.background, customId);
  assert.equal(custom.modal, false);
  assert.deepEqual(custom.focus, { Background: 9 });
  const customRestored = await scenario('wallpaper-custom-restored', 'home');
  assert.equal(customRestored.preferences.background, customId);
  const bundledRestored = await scenario(
    'wallpaper-bundled-restored',
    'settings,down,enter,up,enter,enter',
  );
  assert.equal(bundledRestored.preferences.background, 'forest');
  const clock = await scenario('clock', 'settings,click:900:370');
  assert.equal(clock.subpage, 'Clock');
  assert.equal(clock.focus, 'ClockFormat');
  assert.ok(clock.ui_text.includes('Clock format'));
  assert.ok(clock.ui_text.includes('Time format'));
  const inputsPage = 'settings,down,down,enter,down,down,down,down,enter';
  const inputs = await scenario('inputs-page', inputsPage);
  assert.equal(inputs.subpage, 'Inputs');
  // Inputs opens on Show widget (off): the main screen has no input cards by default.
  assert.equal(inputs.focus, 'InputsToggle');
  assert.equal(inputs.preferences.show_inputs, false);
  // Shown inputs are glyphs at the start of the top bar: Right from the weather button.
  const inputCards = await scenario('inputs-shown', `${inputsPage},enter,home,up,right`);
  assert.equal(inputCards.preferences.show_inputs, true);
  assert.deepEqual(inputCards.focus, { Input: 'com.webos.app.hdmi1' });
  const inputsHidden = await scenario('inputs-hidden', `${inputsPage},enter,home,up`);
  assert.equal(inputsHidden.preferences.show_inputs, false);
  assert.equal(inputsHidden.focus, 'WeatherConfigure');
  const input = await scenario('inputs', `${inputsPage},down,enter,up,enter`);
  assert.equal(input.preferences.input_modes['com.webos.app.hdmi1'], 'Off');
  assert.equal(input.subpage, 'Inputs');
  const closed = await scenario('dialog-closed', 'home,menu,click:100:900');
  assert.equal(closed.modal, false);
  assert.deepEqual(closed.focus, { App: 'youtube.leanback.v4' });
  const fixedTime = ['--time', '2026-10-21T20:41:00+02:00'];
  const one = await scenario('density-1x', 'home', fixedTime);
  const two = await scenario('density-2x', 'home', [...fixedTime, '--capture-scale', '2']);
  assert.deepEqual(two.render.capture.target, { width: 3840, height: 2160 });
  assert.equal(two.render.capture.viewport.scale, 2);
  assert.equal(one.render.capture.viewport.scale, 1);
  assert.equal(two.render.capture.max_logical_font_px, one.render.capture.max_logical_font_px);
  for (const capture of [one.render.capture, two.render.capture]) {
    assert.ok(
      Math.abs(
        capture.max_physical_font_px - capture.max_logical_font_px * capture.viewport.scale,
      ) <= 0.126,
    );
  }
  assert.ok(
    two.render.capture.max_physical_font_px > one.render.capture.max_physical_font_px * 1.9,
  );
  assert.ok(two.render.capture.cached_glyphs > 0);
  assert.ok(two.render.capture.cached_text_runs > 0);
  assert.ok(two.render.capture.glyph_bitmap_bytes > 0);
  assert.ok(two.render.capture.atlas_size >= 1024);
  assert.ok(two.render.capture.text_quads > 0);
  assert.equal(
    two.render.drawable_is_4k,
    two.render.drawable.width >= 3840 && two.render.drawable.height >= 2160,
  );
  const uhd = await sharp(path.join(out, 'density-2x.png')).metadata();
  assert.equal(uhd.width, 3840);
  assert.equal(uhd.height, 2160);
  const low = await sharp(path.join(out, 'density-1x.png'))
    .extract({ left: 84, top: 265, width: 600, height: 96 })
    .resize(1200, 192, { kernel: 'cubic' })
    .raw()
    .toBuffer();
  const high = await sharp(path.join(out, 'density-2x.png'))
    .extract({ left: 168, top: 530, width: 1200, height: 192 })
    .raw()
    .toBuffer();
  assert.notDeepEqual(high, low, '4K vector text must use the drawable pixel scale');
  const hasWallpaper = await fs.access(path.join(out, 'assets/wallpapers/forest@2x.jpg')).then(
    () => true,
    () => false,
  );
  assert.equal(
    two.render.capture.high_resolution_wallpaper,
    hasWallpaper && two.render.graphics.max_texture_size >= 3840,
  );
  const profiled = await scenario('profile', 'home', ['--profile-frames', '24', ...fixedTime]);
  assert.equal(profiled.render.profile.presented, 24);
  assert.ok(profiled.render.profile.pacing.samples > 0);
  assert.ok(profiled.render.profile.cpu_submission.mean_ms >= 0);
  const widgetsPage = 'settings,down,down,enter';
  const widgets = await scenario('widgets-settings', widgetsPage);
  assert.deepEqual(widgets.focus, { Widget: 4 });
  assert.ok(widgets.ui_text.includes('Widgets'));
  for (const title of ['Date', 'Greeting', 'Weather', 'Shortcuts', 'Inputs'])
    assert.ok(widgets.ui_text.includes(title));
  const greetingEditor = `${widgetsPage},down,enter,down,enter`;
  for (const [id, name] of [
    ['Ana', 'Ana'],
    ['Bob', 'Bob'],
    ['long', 'Alexandra Catherine Elizabeth Montgomery'],
    ['wide', 'W'.repeat(40)],
  ]) {
    const named = await scenario(
      `greeting-${id}`,
      `${greetingEditor},type:${name},back,home`,
      fixedTime,
    );
    assert.equal(named.preferences.greeting_name, name);
    assert.ok(named.ui_text.includes(`Good evening, ${name}.`));
    if (name.length > 30) {
      assert.ok(
        named.render.capture.max_logical_font_px < 90.4,
        'Long greetings must shrink instead of wrapping into a cropped second line',
      );
    }
    assert.equal(
      JSON.parse(await fs.readFile(preferences, 'utf8')).greeting_name,
      name,
      'Keyboard completion must save without waiting for another SDL input event',
    );
  }
  const greeting = await scenario('greeting-settings', `${widgetsPage},down,enter,down,down`);
  assert.equal(greeting.subpage, 'Greeting');
  assert.equal(greeting.focus, 'GreetingNameClear');
  assert.ok(greeting.ui_text.includes('Clear name'));

  // App names is one On/Off row in General: titles beneath the focused grid card (tvOS).
  const appNamesRow = 'settings,enter';
  const appNames = await scenario('app-name-settings', appNamesRow);
  assert.deepEqual(appNames.focus, { Setting: 0 });
  assert.ok(appNames.ui_text.includes('App names'));
  assert.equal(appNames.subpage, null);
  assert.equal(appNames.preferences.show_app_names, true);
  const hiddenNames = await scenario('app-names-disabled', `${appNamesRow},enter,home`);
  assert.equal(hiddenNames.preferences.show_app_names, false);
  assert.ok(!hiddenNames.ui_text.includes('Netflix'));
  assert.ok(!hiddenNames.ui_text.includes('YouTube'));
  const shownNames = await scenario('app-names-enabled', `${appNamesRow},enter,home`);
  assert.equal(shownNames.preferences.show_app_names, true);
  // tvOS captions only the focused card (the first app; an earlier scenario reordered).
  assert.equal(
    ['Netflix', 'YouTube'].filter((title) => shownNames.ui_text.includes(title)).length,
    1,
  );

  const renamedInput = await scenario(
    'input-rename-focus',
    `${inputsPage},down,down,enter,down,type:Console,back`,
  );
  assert.equal(renamedInput.focus, 'InputRename');
  assert.equal(renamedInput.preferences.input_names['com.webos.app.hdmi2'], 'Console');
  assert.ok(renamedInput.ui_text.includes('Clear name'));
  const clockEditor = 'settings,enter,down,enter';
  const timeFormat = await scenario('clock-time-format', `${clockEditor},down,enter`);
  assert.equal(timeFormat.subpage, 'TimeFormat');
  assert.deepEqual(timeFormat.focus, { Choice: 0 });
  assert.ok(timeFormat.ui_text.includes('HH:MM'));
  assert.ok(timeFormat.ui_text.includes('HH:MM:SS'));
  const withSeconds = await scenario(
    'clock-time-format-saved',
    `${clockEditor},down,enter,down,enter`,
  );
  assert.equal(withSeconds.subpage, 'Clock');
  assert.equal(withSeconds.focus, 'ClockTimeFormat');
  assert.equal(withSeconds.preferences.show_seconds, true);
  assert.equal(withSeconds.preferences.hour_24, false);
  const clockFocus = await scenario('clock-format-focus', `${clockEditor},enter,down`);
  assert.equal(clockFocus.subpage, 'ClockFormat');
  assert.deepEqual(clockFocus.focus, { Choice: 1 });
  const system = await scenario('system-settings', 'settings,down,down,down,right,down,down');
  assert.deepEqual(system.focus, { Setting: 5 });
  assert.ok(system.ui_text.includes('Refresh apps & inputs'));
  assert.ok(!system.ui_text.includes('Luma update'));
  assert.ok(system.ui_text.includes('Quit app'));
  // Text has one path (the CPU glyph atlas); the Text rendering choice is gone.
  assert.ok(!system.ui_text.includes('Text rendering'));
  assert.equal(system.render.text_rendering.path, 'cpu glyph atlas');
  assert.deepEqual(system.render.text_rendering.requested_samples, [4, 2, 0]);
  assert.ok(system.render.text_rendering.first_frame.glyphs > 0);
  const updatePreview = await scenario('update-preview', 'settings,down,down,down,right', [
    '--test-update-version',
    '99.0.0',
  ]);
  assert.equal(updatePreview.focus, 'InstallUpdate');
  assert.ok(updatePreview.ui_text.includes('Update available'));
  assert.ok(updatePreview.ui_text.some((label) => label.includes('Latest: 99.0.0')));
  assert.ok(updatePreview.ui_text.includes('Install 99.0.0'));
  const afterUpdatePreview = await scenario(
    'update-preview-cleared',
    'settings,down,down,down,right',
  );
  assert.equal(afterUpdatePreview.focus, 'CheckUpdates');
  assert.ok(!afterUpdatePreview.ui_text.includes('Update available'));
  const currentVersion = JSON.parse(
    await fs.readFile(path.join(root, 'package.json'), 'utf8'),
  ).version;
  const latestPreview = await scenario(
    'update-current-preview',
    'settings,down,down,down,right,enter',
    ['--test-update-version', currentVersion],
  );
  assert.equal(latestPreview.focus, 'CheckUpdates');
  assert.equal(latestPreview.update_status, 'checked');
  assert.equal(latestPreview.available_update, null);
  assert.ok(latestPreview.ui_text.includes('No newer version found'));
  assert.ok(latestPreview.ui_text.includes('Check again'));
  assert.ok(latestPreview.ui_text.includes(`Current version: ${currentVersion}`));
  assert.ok(!latestPreview.ui_text.some((label) => label.startsWith('Luma v')));
  // LUMA_MSAA=0 forces a single-sample window: vectors use their analytic ramp, and text
  // (which never depends on MSAA) still draws from the atlas.
  process.env.LUMA_MSAA = '0';
  const singleSample = await scenario('single-sample', 'home');
  delete process.env.LUMA_MSAA;
  assert.deepEqual(singleSample.render.text_rendering.requested_samples, [0]);
  assert.equal(singleSample.render.last_presented.multisample_samples, 0);
  assert.ok(singleSample.render.last_presented.text_quads > 0);
  // Companion HTTP writes are covered by loopback Rust tests. Seed its persisted result
  // here to render the TV's read-only detail/removal flow without a TV keyboard editor.
  const beforeShortcut = JSON.parse(await fs.readFile(preferences, 'utf8'));
  await fs.writeFile(
    preferences,
    JSON.stringify({
      ...beforeShortcut,
      custom_tiles: [
        {
          id: 'custom.sandmann',
          title: 'Sandmann',
          url: 'http://127.0.0.1:9/api/webhook/luma',
          method: 'POST',
        },
      ],
      app_order: [...beforeShortcut.app_order, 'custom.sandmann'],
    }),
  );
  const shortcutsPage = `${widgetsPage},down,down,down,enter`;
  const shortcut = await scenario('shortcut-settings', shortcutsPage);
  assert.equal(shortcut.focus, 'TileEditor');
  assert.equal(shortcut.subpage, 'Shortcuts');
  assert.deepEqual(shortcut.preferences.custom_tiles, [
    {
      id: 'custom.sandmann',
      title: 'Sandmann',
      url: 'http://127.0.0.1:9/api/webhook/luma',
      method: 'POST',
    },
  ]);
  assert.ok(shortcut.preferences.app_order.includes('custom.sandmann'));
  assert.ok(shortcut.ui_text.includes('Configure'));
  assert.ok(shortcut.ui_text.includes('Sandmann'));
  assert.ok(
    !shortcut.ui_text.includes('Type names and addresses there instead of with the remote'),
  );
  assert.ok(!shortcut.ui_text.includes('Add shortcut'));
  assert.ok(!shortcut.ui_text.includes('Request method'));
  // Scenarios share one preferences file, so the tile is removed again here.
  const removed = await scenario('shortcut-removed', `${shortcutsPage},down,enter,down,enter`);
  assert.equal(removed.focus, 'TileEditor');
  assert.equal(removed.subpage, 'Shortcuts');
  assert.deepEqual(removed.preferences.custom_tiles, []);
  assert.ok(!removed.preferences.app_order.includes('custom.sandmann'));
  assert.ok(!removed.inventory.some((app) => app.id === 'custom.sandmann'));
  const tileEditorOpen = `${shortcutsPage},enter,await-tile-editor`;
  const tileEditor = await scenario('tile-editor', tileEditorOpen);
  assert.equal(tileEditor.modal, true);
  assert.equal(tileEditor.focus, 'CloseDialog');
  assert.ok(tileEditor.ui_text.includes('Edit on phone or computer'));
  assert.ok(tileEditor.ui_text.includes('Scan this QR code'));
  assert.ok(tileEditor.ui_text.some((text) => text.startsWith('Keep this window open while')));
  assert.ok(
    tileEditor.ui_text.some((text) =>
      /^http:\/\/127\.0\.0\.1:\d+\/[1-9A-HJ-NP-Za-km-z]{8}$/.test(text),
    ),
  );
  const tileEditorClosed = await scenario('tile-editor-closed', `${tileEditorOpen},back`);
  assert.equal(tileEditorClosed.modal, false);
  assert.equal(tileEditorClosed.focus, 'TileEditor');
  // Grid cards are real pointer targets, also after remote scrolling, and a click never
  // scrolls the clicked card. A fresh file's favourites become the start of the saved order.
  const edgeFavorites = [
    'netflix',
    'youtube.leanback.v4',
    'com.apple.appletv',
    'com.disney.disneyplus-prod',
    'amazon',
    'com.arte.tv',
    'de.zdf.app.zdfm3',
    'ard.mediathek',
    'plex',
  ];
  await fs.writeFile(
    preferences,
    JSON.stringify({ favorites: edgeFavorites, show_weather: false }),
  );
  // Second row, first column: the grid's seventh card (y 628 to 784).
  const secondRow = await scenario('grid-second-row-click', 'home,click:210:700');
  assert.deepEqual(secondRow.focus, { App: edgeFavorites[6] });
  assert.equal(secondRow.scroll, 0, 'Clicking a card must not move it');
  const thirdRow = await scenario('grid-third-row', 'home,down,down');
  assert.equal(thirdRow.scroll, 0, 'Three rows fit beneath the shelf without scrolling');
  const scrolledClick = await scenario('grid-scrolled-click', 'home,down,down,down,click:510:248');
  assert.ok(scrolledClick.scroll > 0, 'Focus below the third row scrolls the shelf away');
  assert.ok(scrolledClick.focus.App, 'The visible card under the pointer takes focus');
  const scrolledBack = await scenario('grid-shelf-returns', 'home,down,down,down,up,up,up');
  assert.equal(scrolledBack.scroll, 0, 'The first row brings the shelf back');
  await fs.writeFile(preferences, JSON.stringify({ show_weather: false }));
  const gradientDisabled = await scenario(
    'wallpaper-gradient-disabled',
    'settings,down,enter,up,up,right,enter',
  );
  assert.equal(gradientDisabled.focus, 'WallpaperGradient');
  assert.equal(gradientDisabled.preferences.wallpaper_gradient, false);
  assert.ok(gradientDisabled.ui_text.includes('Gradient'));
  const gradientRestored = await scenario('wallpaper-gradient-restored', 'home');
  assert.equal(gradientRestored.preferences.wallpaper_gradient, false);
  const gradientEnabled = await scenario(
    'wallpaper-gradient-enabled',
    'settings,down,enter,up,up,right,enter',
  );
  assert.equal(gradientEnabled.preferences.wallpaper_gradient, true);
  const cycleEnabled = await scenario(
    'wallpaper-cycle-enabled',
    'settings,down,enter,up,up,right,right,enter',
  );
  assert.equal(cycleEnabled.subpage, null);
  assert.equal(cycleEnabled.focus, 'WallpaperCycle');
  assert.equal(cycleEnabled.preferences.wallpaper_cycle, true);
  assert.ok(cycleEnabled.ui_text.includes('Rotation'));
  const cycledAfterRestart = await scenario('wallpaper-cycle-restart', 'home');
  assert.equal(
    cycledAfterRestart.preferences.background,
    'san-francisco',
    'enabled rotation advances to the next wallpaper when Luma restarts',
  );
  const cycleDisabled = await scenario(
    'wallpaper-cycle-disabled',
    'settings,down,enter,up,up,right,right,enter',
  );
  assert.equal(cycleDisabled.preferences.wallpaper_cycle, false);
  assert.ok(cycleDisabled.ui_text.includes('Rotation'));
  console.log(
    'Native smoke checks passed: rendering/alpha, navigation, persistence, SDL pointer input, DPI-aware glyphs, 4K capture and presentation profiling.',
  );
} finally {
  await fs.rm(temporary, { recursive: true, force: true });
}
