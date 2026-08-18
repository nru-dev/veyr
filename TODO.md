# Veyr development backlog

## World circles

`WorldCircle` now has a verified build-12340 world-to-screen path and a
terrain/model-aware dynamic implementation through the live client world.

- [x] Static radius-20 player circle through the live `CGCamera` basis and
  field of view.
- [x] Wide cyan diagnostic style with a screen-space glow approximation.
- [x] Hide dynamic-circle segments behind terrain/model/WMO collision without
  borrowing incompatible depth values from the game's D3D pipeline.
- [ ] Implement a real glow pipeline rather than layered translucent lines.
- [x] Add dynamic placement: sample terrain and hold each point at a
  configurable clearance above it.
- [x] Add a read-only WotLK `ADT → MCNK → MCVT` terrain-height decoder; keep
  MPQ lookup and map/tile selection separate from terrain geometry.
- [x] Recover the build-12340 `CGWorldFrame::Intersect` terrain-call ABI and
  isolate it to the injected render thread, so loaded custom maps are queried
  through the client rather than supplied manually.
- [x] Run one in-game vertical-ray probe to validate the terrain collision
  flag and hit record before enabling it for every circle segment.
- [x] Add obstacle-aware dynamic placement: trace each radial segment and
  contour the affected arc around blocking geometry.
- [x] Expose `Static` and `Dynamic { terrain_clearance, avoid_obstacles }` as
  documented, fully implemented render contracts in the developer API.
- [x] Scale visual and native sampling density with radius instead of using a
  fixed segment count; radius 20 now uses 848 visual vertices and 368 live
  terrain/collision samples.
- [x] Use joined screen-space polylines so thick outlines do not leave gaps or
  spikes at terrain and obstacle corners.
- [ ] Turn the player-circle diagnostic into a configurable first-party
  developer plugin and remove its dedicated bootstrap path.
