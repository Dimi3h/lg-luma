---
'lg-luma': minor
---

Optionally replace the TV's own screensaver with Luma's clock on webOS 25/26: `screensaver.sh enable` (as root) bind-mounts a small web app over LG's screensaver, reapplies it at cold boot, and turns Luma's in-app screensaver timer off while it is active. `screensaver.sh disable` restores LG's screensaver.
