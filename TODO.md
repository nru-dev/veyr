# Veyr development backlog

## World circles

`WorldCircle` has a verified build-12340 world-to-screen path. Dynamic
terrain/model placement is implemented behind a deliberately fail-closed
native-collision gate and is disabled until the exact live profile is
revalidated.

- [x] Static radius-20 player circle through the live `CGCamera` basis and
  field of view.
- [x] Wide cyan diagnostic style with a screen-space glow approximation.
- [ ] Validate the native collision profile in the exact target client before
  enabling dynamic terrain/model/WMO placement (the code path is intentionally
  fail-closed and currently renders the safe static ring).
- [ ] Implement a real glow pipeline rather than layered translucent lines.
- [ ] Enable dynamic terrain placement only after the exact native collision
  call contract is revalidated in a dedicated in-game probe.
- [x] Add a read-only WotLK `ADT → MCNK → MCVT` terrain-height decoder; keep
  MPQ lookup and map/tile selection separate from terrain geometry.
- [x] Recover the build-12340 `CGWorldFrame::Intersect` terrain-call ABI and
  isolate it to the injected render thread, so loaded custom maps are queried
  through the client rather than supplied manually.
- [x] Run one in-game vertical-ray probe to validate the terrain collision
  flag and hit record before enabling it for every circle segment.
- [ ] Enable obstacle-aware dynamic placement only after the exact native
  model/WMO collision contract is revalidated in a dedicated in-game probe.
- [x] Expose `Static` and `Dynamic { terrain_clearance, avoid_obstacles }` as
  documented, fully implemented render contracts in the developer API.
- [x] Scale visual and native sampling density with radius instead of using a
  fixed segment count; radius 20 now uses 848 visual vertices and 368 live
  terrain/collision samples.
- [x] Use joined screen-space polylines so thick outlines do not leave gaps or
  spikes at terrain and obstacle corners.
- [ ] Turn the player-circle diagnostic into a configurable first-party
  developer plugin and remove its dedicated bootstrap path.
