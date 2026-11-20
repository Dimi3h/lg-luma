---
'lg-luma': patch
---

Luma as Home stays running across Home presses again: the lifecycle latch that made every Home press start a new Luma (set once after a failed relaunch and never cleared) now lasts at most until the next cold boot or update. A relaunch returns to the main screen with the focused card kept, a cold start shows the wallpaper splash without LG's spinner, a stuck Home is ended after 6 s so the next press starts it afresh, and the Home log keeps the previous process's lines.
