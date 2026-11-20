---
'lg-luma': patch
---

Frame statistics in `luma-diagnostics.json` now measure what matters for a launcher that draws only on change: the time from a key or pointer event to the frame that shows it, and each frame's render time (scene build, submission and swap), with counts of frames over 16.7 ms and 33 ms. The gaps between sporadic frames are no longer reported.
