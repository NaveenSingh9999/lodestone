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
