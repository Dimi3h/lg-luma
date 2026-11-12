// The screensaver takeover's clock page, run headless: a minimal DOM, a fake clock and fake
// timers in a VM context. No browser, TV or network is used.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const html = await fs.readFile(new URL('../tv/screensaver/index.html', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];

function page({ language = 'en-US', start = '2026-11-12T21:07:00' } = {}) {
  let now = new Date(start).getTime();
  let nextId = 1;
  const timers = new Map();
  const listeners = {};
  const element = (id) => ({ id, style: {}, textContent: '', offsetWidth: 360, offsetHeight: 132 });
  const elements = { clock: element('clock'), time: element('time'), date: element('date') };
  const document = {
    hidden: false,
    getElementById: (id) => elements[id],
    addEventListener: (type, listener) => (listeners[type] ??= []).push(listener),
  };
  let closed = 0;
  const schedule = (repeat) => (callback, delay) => {
    const id = nextId++;
    timers.set(id, { callback, delay, due: now + delay, repeat });
    return id;
  };
  const cancel = (id) => timers.delete(id);
  const window = {
    navigator: { language },
    innerWidth: 1920,
    innerHeight: 1080,
    setTimeout: schedule(false),
    setInterval: schedule(true),
    clearTimeout: cancel,
    clearInterval: cancel,
    close: () => closed++,
  };
  class FakeDate extends Date {
    constructor(...args) {
      if (args.length) super(...args);
      else super(now);
    }
    static now() {
      return now;
    }
  }
  let seed = 0.25;
  const random = () => {
    seed = (seed * 9301 + 0.49297) % 1;
    return seed;
  };
  const context = vm.createContext({
    window,
    document,
    Date: FakeDate,
    Math: Object.assign(Object.create(Math), { random }),
    String,
  });
  vm.runInContext(script, context);
  const advance = (ms) => {
    const end = now + ms;
    for (;;) {
      const due = [...timers.entries()]
        .filter(([, t]) => t.due <= end)
        .sort((a, b) => a[1].due - b[1].due)[0];
      if (!due) break;
      const [id, timer] = due;
      now = timer.due;
      if (timer.repeat) timer.due += timer.delay;
      else timers.delete(id);
      timer.callback();
    }
    now = end;
  };
  const fire = (type, extra = {}) => {
    const event = {
      type,
      prevented: false,
      preventDefault() {
        this.prevented = true;
      },
      ...extra,
    };
    for (const listener of listeners[type] ?? []) listener(event);
    return event;
  };
  const position = () => {
    const [, x, y] = elements.clock.style.transform.match(/translate\((\d+)px, (\d+)px\)/);
    return [Number(x), Number(y)];
  };
  return {
    elements,
    document,
    advance,
    fire,
    position,
    timers,
    closed: () => closed,
    api: window.lumaScreensaver,
  };
}

test('only the clock is on a pure black page, with Luma’s colours and a bundled font', () => {
  assert.match(html, /background: #000;/);
  assert.match(html, /#time \{[^}]*color: #bebebe;[^}]*font-size: 76px;/);
  assert.match(html, /#date \{[^}]*color: #7c7c7c;/);
  assert.match(html, /url\('inter-clock\.ttf'\)/);
  assert.match(html, /transition: opacity 350ms ease-in-out;/);
  assert.doesNotMatch(html, /https?:\/\//, 'No network resources');
  const body = html
    .match(/<body>([\s\S]*?)<script>/)[1]
    .replace(/>\s+</g, '><')
    .trim();
  assert.equal(body, '<div id="clock"><div id="time"></div><div id="date"></div></div>');
});

test('a 24-hour time and the date follow the TV language; Dutch is lowercase', () => {
  const dutch = page({ language: 'nl-BE' });
  assert.equal(dutch.elements.time.textContent, '21:07');
  assert.equal(dutch.elements.date.textContent, 'donderdag 12 november');
  const english = page({ language: 'en-GB', start: '2026-03-01T08:05:00' });
  assert.equal(english.elements.time.textContent, '08:05');
  assert.equal(english.elements.date.textContent, 'Sunday, March 1');
  assert.equal(page({ language: 'de-DE' }).elements.date.textContent, 'Thursday, November 12');
});

test('every 5 s the clock fades out for 350 ms, jumps inside the safe area and fades in', () => {
  const tv = page({ language: 'nl-NL' });
  const { clock, time } = tv.elements;
  assert.equal(clock.style.opacity, '1');
  const seen = [tv.position()];
  for (let jump = 0; jump < 20; jump++) {
    // Jumps start every 5 s from the start; the last one ended 350 ms after its start.
    tv.advance(jump === 0 ? 4999 : 4649);
    assert.equal(clock.style.opacity, '1');
    tv.advance(1);
    assert.equal(clock.style.opacity, '0', 'Fading out');
    tv.advance(349);
    assert.equal(clock.style.opacity, '0');
    tv.advance(1);
    assert.equal(clock.style.opacity, '1', 'Faded in at its new place');
    seen.push(tv.position());
    // Between jumps only the jump interval and the next minute change are scheduled.
    assert.equal(tv.timers.size, 2);
  }
  for (const [x, y] of seen) {
    assert.ok(x >= 80 && x + 360 <= 1920 - 80, `x ${x}`);
    assert.ok(y >= 60 && y + 132 <= 1080 - 60, `y ${y}`);
  }
  assert.ok(new Set(seen.map(String)).size > 15, 'The clock keeps moving');
  // The time is current after each jump.
  tv.advance(60_000);
  assert.equal(time.textContent, '21:09');
});

test('any key, click or wheel closes it without acting; early cursor placement does not', () => {
  for (const type of ['keydown', 'pointerdown', 'mousedown', 'click', 'wheel']) {
    const tv = page();
    const event = tv.fire(type, { keyCode: 13 });
    assert.equal(tv.closed(), 1, type);
    assert.ok(event.prevented, `${type} is not passed on`);
    assert.equal(tv.timers.size, 0, 'No timer keeps running');
    assert.equal(tv.api.closed(), true);
  }
  const tv = page();
  tv.fire('mousemove');
  tv.advance(100);
  tv.fire('pointermove');
  assert.equal(tv.closed(), 0, 'The cursor appearing as the window opens is ignored');
  tv.advance(1000);
  tv.fire('pointermove');
  assert.equal(tv.closed(), 1, 'Moving the remote’s pointer closes it');
  tv.fire('keydown');
  assert.equal(tv.closed(), 1, 'Closed once');
});

test('a relaunch restarts the clock and a hidden page runs no timers', () => {
  const tv = page();
  tv.fire('keydown');
  assert.equal(tv.api.running(), false);
  tv.advance(3_600_000);
  tv.fire('webOSRelaunch');
  assert.equal(tv.api.running(), true);
  assert.equal(tv.elements.time.textContent, '22:07');
  assert.equal(tv.elements.clock.style.opacity, '1');
  tv.document.hidden = true;
  tv.fire('visibilitychange');
  assert.equal(tv.timers.size, 0);
  tv.document.hidden = false;
  tv.fire('visibilitychange');
  assert.equal(tv.timers.size, 2);
});

test('the time changes exactly at the minute, between jumps, without moving the clock', () => {
  // 21:07:57: the minute changes 3 s in, before the first jump at 5 s.
  const tv = page({ start: '2026-11-12T21:07:57' });
  const { time } = tv.elements;
  const where = tv.position();
  assert.equal(time.textContent, '21:07');
  tv.advance(3019);
  assert.equal(time.textContent, '21:07');
  tv.advance(1);
  assert.equal(time.textContent, '21:08', 'Shown 20 ms into the new minute');
  assert.deepEqual(tv.position(), where, 'A minute change does not move the clock');
  assert.equal(tv.elements.clock.style.opacity, '1');
  // And again at every following minute, also across the hour and the day.
  const midnight = page({ start: '2026-11-12T23:59:30', language: 'nl-NL' });
  midnight.advance(30_020);
  assert.equal(midnight.elements.time.textContent, '00:00');
  assert.equal(midnight.elements.date.textContent, 'vrijdag 13 november');
  for (let i = 1; i <= 3; i++) {
    midnight.advance(60_000);
    assert.equal(midnight.elements.time.textContent, `00:0${i}`);
  }
});
