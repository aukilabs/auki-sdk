# auki-domain-browser/src

Implementation status for the browser Domain peer adapter.

Currently implemented:

- package scaffold
- Park-compatible contract types, including the current `camera` / `point_cloud` / `joint_encoders` / `audio` / `detection` sensor-kind vocabulary
- global installer
- structured result helpers
- browser identity seed storage seam
- last-six-character peer id display helper
- Discovery HTTP list mapping
- idle participant snapshot shell
- explicit transport-unavailable behavior for real peer operations

Not yet implemented:

- browser-dialable SDK transport
- `/auki/join/0.0.1`
- `/auki/info/0.0.1`
- sensor catalogs
- audio streams
