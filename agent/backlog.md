# Lodestone backlog

1. [M1] Runtime provisioning — Termux repo `.deb` pipeline: index fetch + signature verify, dep resolution incl. ca-certificates-java, truststore placement, ELF gate on every lib, `java -version` + TLS smoke test (spec 4.2)
2. [M2] PaperMC Fill v3 provider + Folia/Purpur stubs behind same Provider trait (spec 4.1)
3. [M3] Supervisor live loop: stdin/stdout pump, Done detection, backoff restarts, crash windows (spec 4.4)
4. [M4] UniFFI FFI surface (section 3.3 commands/events) + Kotlin adapter skeleton
5. [M5] Android `:server` service process + dlopen/JLI_Launch shim (spec 5; soak test acceptance gate)
6. Sleep-when-empty proxy, tunnel verdict, remote API — after M5 (spec 4.11-4.13)

DONE M2 (6d58b60): Paper Fill v3 provider, sha256 gate-first, STABLE-only, fixtures+8 tests
DONE M3 (5fcd1ee): supervisor + real POSIX exec
DONE M4 (0b0addc): FFI surface + UDL
DONE M5 (CI): Android shell scaffold + native build + APK, all green in GitHub Actions

DONE: config, backup, content, schedule, crash, tunnel — all CI green
DONE: sleep-when-empty proxy (4.12) — MC status/login protocol, wake hook, idle tracker, PROXY v1
DONE: remote management API (4.13) — HTTP+WS, bearer token auth, attempt lockout, LAN-only
DONE: Fabric/Quilt/Forge/NeoForge catalog providers (4.1) — live-API verified, sha1/sha256 gate-first, fixtures+tests
DONE: Engine FFI wiring — lodestone_start/stop_sleep_proxy, lodestone_start/stop_remote_api, shared broadcast event channel, both CI/Android green
DONE: Android spec 5 surfaces — notification with status + start/stop actions, Quick Settings tile, Glance home-screen widget (all read from core state via StateStore)

REMAINING: M5 depth items (JLI_Launch dlopen, RCON metrics, Geyser crossplay).
