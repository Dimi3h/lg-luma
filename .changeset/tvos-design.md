---
'lg-luma': minor
---

Restyle the launcher after current Apple tvOS (tvOS 26/27, Liquid Glass). Every feature, preference and navigation rule is unchanged; only presentation and motion differ.

- Focus lifts and grows cards (10%), rows and buttons with a deep shadow instead of drawing a ring, and the lift, scrolling and page switches ease (about 200–250 ms). Reduced motion snaps all of it, and hit regions never move.
- Layout and type follow the tvOS guidelines: 80 × 60 safe area, six 260 × 156 cards per row with 40/100 spacing, and the tvOS text styles from Title 1 down to Caption 2, with nothing smaller.
- Liquid Glass-style panels (blurred background, top highlight and rim) for the tab bar, Apps tab bar and search, settings sidebar and rows, context menus, alerts, keyboard and notices. Bundled backgrounds and new uploads get a small blurred copy for this; older uploads use a translucent fallback.
- Home: tvOS-style top shelf, the favourites row on the grid with the seventh card cut off at the edge, and inputs as 5:3 cards with a status dot and their full status beneath when focused.
- App names, when enabled, now appear only beneath the focused card, as on tvOS.
- Apps: a segmented tab bar with a new **All** segment (in place of the clear-filter button) and a glass search field; Up from the first row returns to the selected segment.
- Settings: a glass sidebar with a title, rounded rows, and the section and subpage path as a title line above the content.
- Overlays: the options popup becomes a context menu beside the lifted card, deletion a centred alert with side-by-side buttons, notices a banner that slides down below the tab bar, and Recent apps a single row of cards with the focused one titled.

The new **All** label is machine-translated for every language except English.
