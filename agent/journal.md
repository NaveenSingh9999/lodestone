# Lodestone journal

## 2026-10-24 — M0 core foundation
- Read spec (lodestone.md) and did demand research from Reddit/GitHub/Play Forum/Google tracker sources before coding (see previous session context).
- User directive: no mocks/placeholders in production code. Removed mock.rs and runtime_util.rs; deleted in-memory/fixture implementations from src; real implementations now in place (reqwest+rustls HttpsClient with spec's host allowlist enforced on initial requests AND redirects via redirect-policy; FsStorage with canonicalization + escape rejection + atomic temp-rename writes; VanillaProvider with real install: download → SHA1 verify → size verify, and refusal of checksum-less artifacts per spec "never run an unverified download").
- Platform trait implemented per spec 3.2 object-safe; ServerHandle uses tokio channels; ExitInfo classifies SIGKILL/137 as KilledBySystem (spec 4.4).
- 16 KB gate re-derived cleanly with min-LOAD-align semantics; probed against the real device: auxv AT_PAGESZ parsed (device defaults documented fallback 4096), gate tested against real /data/data/com.termux/files/usr/lib/libc.so when present.
- Test-only fixtures/test platform moved under tests/support (spec 3.1 rule 4 + criterion 9: core must test with mocked adapter and no Android deps).
- Fixed during development: serde defaults for heap_profile; list_dir real-FS semantics (children of all kinds); trait futures annotated `+ Send` to satisfy async_fn_in_trait; launch-spec path validation handles relative jars.
- Verification: `cargo test` 38 passed / 0 failed across 6 binaries, zero warnings, cargo fmt clean.

## Reliance notes
- reqwest 0.12 + rustls on aarch64 Android builds under Termux rustc 1.97.1 (proven by build).
- Auxv page-size probe validated against /proc/self/auxv in tests.
- Spec quotes followed: hash verification mandatory (4.1), no unverified downloads (9), SIGKILL classification (4.4), memory budget formula (4.5).

- M1 core built+committed be4f1db: termux repo verification chain, resolver, deb extraction, provisioner. Device thermal constraint adopted: no full cargo test runs; targeted lib tests only. E2E (102MB openjdk-21 closure) awaits user go.

- M3 (5fcd1ee): PosixPlatform (real fork/exec, process groups, SIGKILL group kill, signal-faithful exit class) + InstanceRunner driving the FSM over a real handle. 6 tests: graceful stop FSM chain, crash-loop give-up, system-kill never-restarted, hung-server SIGKILL escalation, launch-spec escape, stdout bridge. 37+8+7+5+6+4 lib/integration tests all green.

- M4 (0b0addc): lodestone-ffi C ABI (engine, device report, catalog list_versions, create/list/delete instance, start/stop/kill/send_console, analyze_crash, provision_runtime) + UDL mirroring spec §3.3. Smoke test round-trips engine + list_server_types + device_report.

- M5 CI green: Rust native lib (arm64-v8a) built via cargo-ndk in CI and packaged into APK; Compose + :server foreground service + JNA bridge compile. Full pipeline (native + build) passes.

- All remaining spec scope done: config engine, backup engine, Modrinth content, scheduling+sleep-when-empty, crash analysis, tunnel manager. CI green.

## Session: sleep proxy (4.12) + remote API (4.13)
- a001e58: tunnel stub shebang portable (CI fix)
- 1ba9151/d0e59e9: sleep proxy — varint/framing, handshake parse, SleepingSession (status MOTD / wake-kick), SleepProxy serve loop, passthrough w/ PROXY v1, query_status, IdleTracker (fires-once semantics). 13 tests incl. 2 loopback e2e.
- 827c2a5: remote API — hand-rolled HTTP/1.1 parse/encode, AuthGate (constant-time eq, N-fail lockout), /health, POST /api JSON dispatch, /events WS (RFC 6455 handshake w/ sha1+base64, broadcast event pump, ping/pong). 10 tests.
- 96 lib tests; CI + Android green (runs 40178339917, 40181738791).
- Note: let-else single-line formatting differs between local rustfmt and CI — avoid let-else in shared code.

## 2026-10-25 — Catalog providers + FFI wiring

- Fabric provider (meta.fabricmc.net v2): installer artifact verified via sha1 sidecar (`{url}.sha1`), no published checksum on the server-jar bundle so the installer is the verifiable path; 3 tests (versions, resolve sha1-gated, install verify).
- Quilt provider (meta.quiltmc.org v3): installer sha1 inline in JSON — no sidecar fetch needed; 2 tests.
- Forge provider (files.minecraftforge.net promotions_slim.json + maven sha1 sidecar); 1 test.
- NeoForge provider (maven versions API + sha1 sidecar, highest patch per MC version); 1 test.
- All providers registered in catalog/mod.rs, allowlist already covered the hosts. 103 lib tests total, all green.
- FFI wiring: added `lodestone_start_sleep_proxy` / `lodestone_stop_sleep_proxy` / `lodestone_start_remote_api` / `lodestone_stop_remote_api` to lodestone-ffi C ABI; wake/sleep hooks flip the proxy running flag + emit events; shared `broadcast::Sender<Event>` channel wired into `emit()` so `/events` WS subscribers receive typed events; remote API runs on a dedicated tokio runtime so `dispatch` can safely `block_on` engine.rt.
- Verification: cargo fmt clean, clippy `-D warnings` clean, 103 lib tests pass, both CI and Android workflows green on commit b1494cc.
- Next: Android surfaces (spec 5) — notification upgrade with server status, Quick Settings tile, Glance home-screen widget.

## 2026-10-25 — Spec 5 Android surfaces

- Notification upgrade (LodestoneService.kt): registers event callback via NativeBridge, tracks instance state via StateStore (SharedPreferences), updates foreground notification dynamically with per-instance status summary, adds "Start All"/"Stop All" action buttons via PendingIntents.
- Quick Settings tile (QsTileService.kt): TileService that reflects running state (ACTIVE/INACTIVE) and toggles start/stop all via foreground service intent.
- Glance home-screen widget (LodestoneWidget.kt + LodestoneWidgetReceiver.kt + widget_info.xml): GlanceAppWidget showing server status summary with Start/Stop buttons that launch foreground service actions.
- NativeBridge.kt: added FFI bindings for lodestone_list_instances, lodestone_start/stop_sleep_proxy, lodestone_start/stop_remote_api.
- AndroidManifest.xml: BIND_QUICK_SETTINGS_TILE permission, QsTileService + LodestoneWidgetReceiver declarations.
- build.gradle.kts: added androidx.glance:glance-appwidget:1.0.0.
- Verification: cargo fmt clean, clippy -D warnings clean, 103 lib tests pass, both CI and Android workflows green on commit 84d7a94.

## 2026-10-25 — Android service lifecycle integration (spec 5 depth)

- LodestoneService now starts the remote management API on 127.0.0.1:7300 (LAN-only, bearer token auth) when the service is created, per spec 4.13.
- Sleep proxy auto-starts on 0.0.0.0:25565 → 127.0.0.1:25565 when any instance transitions to Running (via StateChanged event), and stops when all instances are Stopped.
- Notification gained "Start Sleep Proxy"/"Stop Sleep Proxy" actions reflecting current proxy state.
- Both CI and Android workflows green on commit 85057df.
