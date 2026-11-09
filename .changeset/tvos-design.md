---
'lg-luma': minor
---

Restyle the launcher after current Apple tvOS (tvOS 26/27, Liquid Glass) and turn it into one tvOS-style main screen. Features and preferences are kept; inputs move behind a preference.

- **One screen**: the Home/Apps/Settings tab bar and the TV-settings button are gone. Beneath the clock and the date, greeting and weather (the top shelf) is one six-column grid of every app: a **Search** card, then your pinned favourites in their saved order, then every other app by title, and a **Settings** card last that opens Settings full screen (Back returns to that card). Pinning moves an app into the favourites block; hold OK on a favourite to reorder it.
- **Shelf and scrolling**: focusing below the first row scrolls the grid up and dims the shelf away; returning to the first row brings it back. Up/Down follow the grid's six columns, including offscreen rows.
- **Launching and options**: OK launches the focused app, as on tvOS. Menu, or holding OK on an app that is not a favourite, opens its options.
- **Search**: the Search card opens the keyboard and its matches replace the grid until Back. Search also finds hidden apps, which is now how they are shown again. Category filters were removed.
- **Inputs**: HDMI inputs no longer appear on the main screen by default. **Settings › Widgets › Inputs › Show widget** adds them as a row of cards under the shelf; the remote's input button and the Inputs settings stay as before.
- **App names**: one On/Off setting in General; titles appear only beneath the focused card.
- **Focus and motion**: the focused card, row or button lifts and grows with a deep shadow instead of drawing a ring, on a light spring with a slight overshoot; scrolling and page switches ease (about 200–250 ms). Reduced motion snaps all of it, and hit regions never move.
- **Focus tilt** (Settings › General, on by default, off under Reduced motion): the focused card tilts towards the Magic Remote pointer or briefly towards a d-pad press, its layers separating (parallax) with a moving glint.
- **Sounds** (Settings › General, on by default): soft clicks on focus moves, selection and Back, synthesized at startup; silent if no audio device is available.
- **Favorite badges** (Settings › General, off by default): the heart on pinned favourites is now optional.
- **Typography and corners**: Inter Display (from the bundled Inter's optical-size axis) for large titles, lighter tracking, the date in sentence case, and continuous (superellipse) corners. Resting cards have no rim; it appears with the lift.
- **Layout and type** follow the tvOS guidelines: 80 × 60 safe area, 260 × 156 cards with 40/100 spacing, and the tvOS text styles from Title 1 down to Caption 2, with nothing smaller.
- **Liquid Glass-style panels** (a lighter, more saturated blurred background, a soft top highlight and a thin bright top edge) for the Search and Settings cards, the settings sidebar and rows, context menus, alerts, keyboard and notices. Bundled backgrounds and new uploads get a small blurred copy for this; older uploads use a translucent fallback.
- **Settings**: a glass sidebar with a title, rounded rows, and the section and subpage path as a title line above the content.
- **Overlays**: the options popup becomes a context menu beside the lifted card, deletion a centred alert with side-by-side buttons, notices a banner that slides down at the top, and Recent apps a single row of cards with the focused one titled.

The new **Search**, **Focus tilt**, **Sounds** and **Favorite badges** labels are machine-translated for every language except English.
