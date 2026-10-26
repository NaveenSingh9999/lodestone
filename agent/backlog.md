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
DONE: Android service lifecycle integration — LodestoneService starts remote API on 127.0.0.1:7300 on create; auto-starts sleep proxy (0.0.0.0:25565 → 127.0.0.1:25565) when instance goes Running, stops when all Stopped; notification actions to start/stop sleep proxy

DONE: RCON metrics (4.3 depth) — Source RCON client + MetricsCollector + InstanceRunner callback, all CI/Android green
DONE: JLI_Launch dlopen (spec §5 depth) — AndroidPlatform dlopen's libjli.so + calls JLI_Launch inside :server service process, piped stdio, wired into FFI via cfg(target_os = "android"), compiles for aarch64-linux-android, 3 tests
DONE: Geyser/Bedrock crossplay (4.11 optional) — crossplay.rs installs Geyser/Floodgate/ViaVersion/ViaBackwards via Modrinth v2 into plugins/ (paper-family) or mods/ (fabric/forge/etc.), server-type-aware slugs, 7 tests
DONE: Geyser wired to Android UI — AccessScreen.kt with crossplay toggle, NativeBridge.enableCrossplay binding, MainActivity updated with engine pointer + Access navigation
DONE: App logo — /sdcard/Download/logo.svg converted to mipmap PNGs at all densities (mdpi/hdpi/xhdpi/xxhdpi/xxxhdpi) + round variants; manifest icon/roundIcon attributes set

REMAINING: Optional end-to-end device testing on real Termux device.

DONE: Silent crash fix — added res/values/strings.xml + colors.xml (values dir was empty), error handling in MainActivity and LodestoneService, notification icon (ic_notification drawable). App icon updated from /sdcard/Download/Apple-Symbols-Font/ PNG. Production APK rebuilt: 13MB at /sdcard/Download/lodestone-release.apk.

DONE: Error handling for silent crashes — try-catch in MainActivity and LodestoneService, error display in UI, notification icon (ic_notification drawable). Production APK rebuilt: 13MB at /sdcard/Download/lodestone-release.apk.

DONE: Production APK built and signed — 12MB, available at /sdcard/Download/lodestone-release.apk (keystore: lodestone.keystore, pass: lodestone123, alias: lodestone).
