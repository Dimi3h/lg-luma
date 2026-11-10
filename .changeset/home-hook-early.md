---
'lg-luma': patch
---

Home takeover starts sooner after a cold boot: the boot hook is now `10-luma-home`, so Homebrew Channel runs it before other apps' startup hooks (an existing `64-luma-home` link is replaced by `enable` or at the next boot), it no longer waits a fixed 5 seconds, and the takeover's checks poll every 100 ms instead of every second. Each step is logged with its timing to `/var/lib/luma-home/boot.log`. After an update, the next cold boot copies the installed app's newer Home scripts when they are owned by root; otherwise run `home.sh enable` again.

Overlays have no motion either: the context menu (also from a long press), alerts, the keyboard and Recents open and close at once, complete in their first frame, with focus inside them instant as in Settings. While OK is held, the long-press menu is prepared off screen so its labels are ready when the hold fires.
