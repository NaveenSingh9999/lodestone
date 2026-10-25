# SUMMARY — what happened while I was away

## Done (2026-10-25 run)
- Catalog provider spec 4.1 complete: Fabric (meta.fabricmc.net v2, sha1 sidecar), Quilt (meta.quiltmc.org v3, inline sha1), Forge (promotions_slim.json + maven sha1 sidecar), NeoForge (maven versions API + sha1 sidecar) — all live-API verified against real endpoints, fixtures captured, checksum-gated installs, 7 new tests. Total lib tests: 103, all green.
- FFI wiring for spec 4.12/4.13: `lodestone_start_sleep_proxy`, `lodestone_stop_sleep_proxy`, `lodestone_start_remote_api`, `lodestone_stop_remote_api` exposed via the lodestone-ffi C ABI; wake/sleep hooks flip proxy behavior + emit events; shared broadcast event channel feeds `/events` WS subscribers; remote API runs on a dedicated tokio runtime.
- Both GitHub CI (fmt/clippy/test) and Android workflows green on commit b1494cc.

## In progress
- Nothing half-done; commit b1494cc is clean and CI-green.

## Queued next (agent/backlog.md)
- Android surfaces: notification upgrade with server status, Quick Settings tile, Glance home-screen widget (spec 5).
- M5 depth items (deferred): JLI_Launch dlopen, RCON metrics, Geyser/Bedrock crossplay.

## Blocked / needs human decision
- None.
