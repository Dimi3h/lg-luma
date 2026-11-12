---
'lg-luma': minor
---

Record rolling frame statistics while Luma is in front (frames, interval mean/p95/max, intervals above 20 ms and 33 ms, partial vs full redraws, idle time) in `luma-diagnostics.json`, written off the SDL thread at most once a minute and at exit.
