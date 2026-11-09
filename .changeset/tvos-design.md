---
'lg-luma': minor
---

Restyle the launcher after current Apple tvOS (tvOS 26/27, Liquid Glass) and turn it into one tvOS-style main screen. Features and preferences are kept; inputs move behind a preference.

- **One screen**: the Home/Apps/Settings tab bar and the TV-settings button are gone. Beneath the clock and the date, greeting and weather (the top shelf) is one six-column grid of every app: your pinned favourites in their saved order, then every other app by title, and a **Settings** card last that opens Settings full screen (Back returns to that card). Pinning moves an app into the favourites block; hold OK on a favourite to reorder it.
- **Compact shelf and scrolling**: the date shares the clock's line, with the greeting and one row of weather directly beneath, so three rows of cards (18 cards) fit on the screen without scrolling. With more rows, the grid scrolls by whole rows past the third and the shelf collapses; returning to the first row brings it back. Up/Down follow the grid's six columns, including offscreen rows.
- **Launching and options**: OK launches the focused app, as on tvOS. Menu, or holding OK on an app that is not a favourite, opens its options.
- **No app search**: there is no Search card or app search keyboard (the keyboard stays for the weather location and other text fields). Hidden apps are listed in **Settings › General › Hidden apps**, where OK shows one again; the list has an empty state when nothing is hidden. Category filters were removed.
- **Inputs**: HDMI inputs no longer appear on the main screen by default. **Settings › Widgets › Inputs › Show widget** adds them as a row of cards under the shelf; the remote's input button and the Inputs settings stay as before.
- **App names**: one On/Off setting in General; titles appear only beneath the focused card.
- **Focus and motion**: the focused card, row or button lifts and grows with a deep shadow instead of drawing a ring, on a light spring with a slight overshoot; scrolling and page switches ease (about 200–250 ms). Reduced motion snaps all of it, and hit regions never move.
- **Focus tilt** (Settings › General, on by default, off under Reduced motion): the focused card tilts towards the Magic Remote pointer or briefly towards a d-pad press, its layers separating (parallax) with a moving glint.
- **Control Center**: OK (or a hold) on the clock, a hold on the shelf's weather button, or a click on the clock opens a tvOS-style glass panel from the right with Sleep (asks for a second OK), volume and mute, sound output, the inputs and the network status. TV calls run off the SDL thread; failures show a quiet notice.
- **Night wallpaper** (off by default): a background's options offer **Use at night**; between sunset and sunrise at the weather location (21:00–07:00 without one) it replaces the chosen wallpaper.
- **Text size** (Settings › General): Default or Larger (about 1.15× body text; titles unchanged), with the three-row main screen still fitting.
- **Jiggle**: while you move a favourite, all favourites wobble slightly, as tvOS icons do; none under Reduced motion.
- **Appearance** (Settings › General): Dark (default, unchanged), Light (lighter wallpaper and glass, dark text) or Automatic, which switches at sunrise and sunset at the weather location (07:00–21:00 without one). Forecasts now include today's sunrise and sunset.
- **Screensaver** (Settings › General, 10 minutes by default; Off, 5, 15 or 30 also): a slow Ken Burns over the bundled and uploaded wallpapers with an optional clock, only while Luma is in front; any key or pointer movement ends it. Still pictures under Reduced motion.
- **App artwork** (Settings › Widgets, on by default): the focused first-row app's background or splash image (else its icon) washes softly behind the top shelf, crossfading between apps; Reduced motion swaps instantly.
- **Favorite badges** (Settings › General, off by default): the heart on pinned favourites is now optional.
- **Typography and corners**: Inter Display (from the bundled Inter's optical-size axis) for large titles, lighter tracking, the date in sentence case, and continuous (superellipse) corners. Resting cards have no rim; it appears with the lift.
- **Layout and type** follow the tvOS guidelines: 80 × 60 safe area, 260 × 156 cards with 40/100 spacing, and the tvOS text styles from Title 2 down to Caption 2, with nothing smaller.
- **Liquid Glass-style panels** (a lighter, more saturated blurred background, a soft top highlight and a thin bright top edge) for the Settings card, the settings sidebar and rows, context menus, alerts, keyboard and notices. Bundled backgrounds and new uploads get a small blurred copy for this; older uploads use a translucent fallback.
- **Settings**: a glass sidebar with a title, rounded rows, and the section and subpage path as a title line above the content.
- **Overlays**: the options popup becomes a context menu beside the lifted card, deletion a centred alert with side-by-side buttons, notices a banner that slides down at the top, and Recent apps a single row of cards with the focused one titled.

- **Languages**: this fork ships English and Dutch only; a saved choice of another language loads as English. The Noto Chinese and Korean fallback fonts are gone with them, which makes the IPK about 28 MB smaller; characters Inter lacks draw as its missing-glyph box. The Dutch strings were reviewed by hand rather than machine-translated.
- **Updates** come only from the `Dimi3h/lg-luma` fork's releases and Homebrew feed, never from upstream; until the fork publishes its first release the check quietly finds nothing newer.
