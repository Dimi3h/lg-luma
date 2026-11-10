---
'lg-luma': patch
---

The clock screensaver no longer jumps abruptly: every 5 seconds the clock fades out at its old place and in at the new one (350 ms each, eased; just opacity, no sliding, never brighter than its resting grey). Under Reduced motion it still jumps instantly. The background stays pure black, and frames are drawn only during the fades and at minute changes.
