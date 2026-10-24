# Lodestone: Concept Specification

> Audience: an AI coding agent. This document is the source of truth for what to build and why. Where it states a decision, follow it. Where it says "verify", confirm against live documentation before coding, because external APIs change.

## 1. Product

Lodestone is a Minecraft Java server manager. The user installs any server type and version, and Lodestone downloads, configures, tunes, runs, monitors, and backs up the server with minimal input.

**Current target: Android only** (non-root, arm64). Launch scope is Minecraft 1.17 and newer (Java 17, 21, and newer), which the primary runtime source covers. Older versions (Java 8) are best-effort through fallback runtime sources. The architecture must allow Windows (WinUI 3) and macOS (SwiftUI) to be added later by writing only a native UI shell and a thin platform adapter. No core logic may change for a new platform.

Not affiliated with Mojang or Microsoft. Do not use "Minecraft" in the product name or app icon. Do not bundle Mojang assets. Server JARs are always downloaded from their official public sources at the user's request.

## 2. Decisions

| Decision | Choice | Reason |
|---|---|---|
| Shared core language | **Rust** | One native library with first-class FFI to Kotlin, Swift, and C#. Small, no GC, good on phones. |
| FFI layer | **UniFFI** | Generates Kotlin and Swift bindings from one interface definition. C# via `uniffi-bindgen-cs`. |
| Async runtime | **tokio** | Standard, works on Android. |
| Android UI | **Kotlin + Jetpack Compose**, Glance for widgets | Native, modern. |
| Future UIs | WinUI 3 (C#), SwiftUI | Native on each platform, same core. |
| Distribution | **Sideload / GitHub Releases / F-Droid only. Not Google Play.** | Downloads and runs server JARs and a JVM, which Play policy is hostile to. |
| Android target | `targetSdk 34` or current, `minSdk 26`, `arm64-v8a` only | Research shows `targetSdk 28` is not needed (see 4.2 and 5). Staying on a current target avoids Android's rising minimum-install floor. 32-bit devices cannot run useful heap sizes. |
| Java runtime | **Reuse existing maintained Android JDK builds. Lodestone builds and hosts no JDKs.** Primary source: Termux's public package repository. Fallback: community CI builds on GitHub Releases. | Avoids a lifetime of maintaining OpenJDK-for-Android builds. Termux ships fresh builds continuously. |
| Server process model | **The JVM runs inside a dedicated Android service process (`android:process=":server"`), loaded in-process through `dlopen` of the JDK's launcher library and `JLI_Launch`.** Fallback: a launcher executable shipped in the APK's native library directory that `execve`s the JVM. | Android blocks `execve` of files in app storage at modern targets but allows `dlopen` of libraries from it. A service process is a normal Android process, not a forked child, so it should avoid the phantom process killer. Process death is the clean stop/restart boundary. Both techniques are used by existing apps (see 2.1 and 4.2). |
| Foreground service type | `specialUse` | `dataSync` has a runtime timeout on recent Android. `specialUse` does not. |
| Config format | TOML per instance | Human-readable, easy to diff. |
| Backups | tar + zstd | Fast, good ratio, low CPU. |
| Console channel | Process stdin/stdout for console, RCON for control and metrics | stdin is reliable for commands, RCON is structured. |
| Public access | **Embed the `playit-agent-core` Rust crate (BSD-2-Clause) in the core.** LAN address always shown. | Free tunnel, no router config, no separate binary to execute. Both existing apps use playit.gg. |
| Idle handling | Core-level **sleep-when-empty / wake-on-join proxy** | Saves battery and heat. Existing apps and the `lazymc` project prove the pattern. |
| Remote control | Core exposes an **optional authenticated HTTP + WebSocket API** | Users want to manage the phone from a laptop. Also lets future desktop apps control a phone-hosted server. |
| Telemetry | None | Privacy. |

If a future Android version blocks executing from app storage at the chosen target level, the fallback is to package the launcher and runtime as native libraries inside the APK. Isolate this behind the `Platform` interface so the switch is contained.

### 2.1 Verification results

Research done before writing this spec. "Verified" means confirmed from primary or project sources. Anything still needing a physical device is listed under residual tests and must be treated as an acceptance gate.

| Item | Result | Evidence | Residual test |
|---|---|---|---|
| JVM loaded in-process on Android | **Verified at scale** | PojavLauncher `dlopen`s `libjli.so` from app storage and calls `JLI_Launch`. Its manifest runs the game in a separate `:game` process and its game service is a foreground service of type `specialUse`. This is the same design as this spec. | Server workloads differ from client workloads. Run a multi-hour soak test with a Termux-built JDK. |
| Phantom process killer vs a service process | **Supported by design, not device-tested** | The Android change that introduced the killer targets child processes spawned by `Runtime.exec` that the framework cannot track, and kills them when CPU-heavy and the parent app is in the background. A service declared with `android:process` is a framework-tracked app process, not an untracked child. | Run a CPU-heavy server for hours with the screen off on Android 14, 15, and 16, and confirm no SIGKILL. The low-memory killer still applies. |
| Running without `targetSdk 28` | **Verified** | MiniCraft targets API 34 and runs a JVM. Android 15 only blocks installs below API 24, so 28 would install today, but that floor keeps rising. | Run on Android 15 and 16. |
| Termux JDK truststore | **Root cause found, fix defined** | Termux's `ca-certificates` build creates a Java keystore named `jssecacerts` in the JDK's `lib/security` directory, shipped as the `ca-certificates-java` package. If only the JDK package is extracted, the truststore is missing and TLS (including online-mode authentication) fails. | The smoke test in 4.2 must pass a real TLS connection. |
| Java versions in Termux | **17 and 21 confirmed in the repo listing. 25 is used by a Termux install script. Java 8 not found.** | Repo directory listings and a Termux server script. | Check for `openjdk-25` in the live repo index before relying on it. |
| `playit-agent-core` on Android | **Likely, not directly confirmed** | docs.rs only lists x86 Linux build targets, which is a docs-build limitation. MiniCraft ships `playitd` as an arm64 Android native library, and MC Host ships a `playitd` built for Android, so the playit codebase builds for Android. The playit repository README marks only some agent releases as non-deprecated. | Build the crate for `aarch64-linux-android` in CI. Pin the version the current playit agent release uses. Fallback in 4.11. |
| 16 KB page-size devices | **New risk found** | Android 15 and later can run with 16 KB pages. 64-bit native libraries must have 16 KB ELF segment alignment, otherwise `dlopen` fails with a program-alignment error. Prebuilt JDK libraries loaded from app storage are not covered by build tooling. | Provisioning must gate on page size (4.2 step 5). Test on a 16 KB-page device or emulator image. |

## 3. Architecture

Three layers. Dependencies point downward only.

```
Native UI shell  (Compose / WinUI / SwiftUI)   renders state, sends commands
        |
Platform adapter (Kotlin / C# / Swift)          implements the Platform interface
        |
Lodestone Core   (Rust)                         all logic, no platform imports
```

### 3.1 Rules

1. **All product logic lives in the core.** The UI and adapter contain none: no version resolution, no flag generation, no scheduling, no parsing.
2. **The UI is a pure renderer.** It subscribes to state and events from the core and sends commands. It holds no business state beyond transient view state (scroll position, text input).
3. **The core never imports platform APIs.** Anything platform-specific is reached through the `Platform` interface.
4. **All network and filesystem access in the core goes through traits** (`HttpClient`, `Storage`) so they can be mocked in tests.
5. **Every long operation reports progress** as events and is cancellable.
6. **Errors are typed** (an enum per module), never strings. The UI maps them to localized messages.

### 3.2 Platform interface

Implemented by each platform adapter. Keep this surface small. Adding a platform means implementing exactly this.

```
trait Platform {
    // Locations
    fn data_dir() -> Path                       // private root for instances, runtimes, backups
    fn export_dir_handle() -> Option<Handle>    // user-visible export target (SAF on Android)

    // Keep-alive and lifecycle
    fn begin_keepalive(reason)                  // Android: foreground service + wake/Wi-Fi locks
    fn end_keepalive()
    fn request_background_exemptions() -> ExemptionState   // battery optimization, etc.
    fn run_on_boot(enabled: bool)

    // Device telemetry
    fn memory_info() -> MemInfo                 // total, available, low-memory flag
    fn cpu_info() -> CpuInfo                    // core count, max freq
    fn thermal_status() -> Thermal              // None/Light/Moderate/Severe/Critical + temp C
    fn power_status() -> Power                  // battery %, charging, power-save mode
    fn network_info() -> NetInfo                // local IPs, Wi-Fi/cellular/hotspot

    // Surfaces
    fn show_status(summary: StatusSummary)      // ongoing notification / tray / menu bar
    fn notify(event: UserNotification)

    // Server process hosting
    fn launch_server(spec: LaunchSpec) -> ServerHandle
        // ServerHandle: stdin sink, stdout/stderr stream, exit future (code + signal), kill()
        // Android: starts the :server service process and hosts the JVM in it.
        // Desktop platforms: plain std::process child. The core never spawns the JVM itself.

    // Runtime provisioning
    fn runtime_sources(java_major) -> [RuntimeSource]    // ordered sources for this OS/arch, from the runtime catalog
}
```

The core never spawns the JVM directly. It asks the platform through `launch_server`, because how a JVM may be started differs sharply per OS (on Android it is the central problem). Platforms also differ in which runtime sources they use.

### 3.3 Core interface to the UI

UniFFI exposes two things.

**Commands** (async functions) and **state + events** (callback interface the UI implements).

```
Core
  // instances
  create_instance(spec: InstanceSpec) -> InstanceId       // runs install, emits progress
  list_instances() -> [InstanceSummary]
  delete_instance(id)
  import_instance(path) -> InstanceId

  // lifecycle
  start(id) / stop(id) / restart(id) / kill(id)
  send_console(id, line)
  accept_eula(id)                                         // explicit user action only

  // catalog
  list_server_types() -> [ServerType]
  list_versions(type, filter) -> [VersionInfo]            // cached, refresh on demand

  // config
  get_config(id) / set_config(id, patch)                  // validated, applies on next start
  apply_tuning_profile(id, profile)

  // backups
  create_backup(id) / list_backups(id) / restore_backup(id, backup) / export_backup(id, backup)

  // content
  search_content(id, query, kind) / install_content(id, ref) / list_content(id)
  check_content_updates(id) / update_content(id, refs)
  install_modpack(id, source)

  // schedule
  list_tasks(id) / set_task(id, task) / remove_task(id, task_id)

  // players
  players(id) / whitelist(id, ...) / ops(id, ...) / bans(id, ...)

  // diagnostics
  analyze_crash(id) -> CrashReport
  device_report() -> DeviceReport                         // RAM class, thermal, capacity estimate

Event stream (UI implements EventListener)
  StateChanged(id, InstanceState)
  Progress(op_id, label, fraction, bytes)
  ConsoleLine(id, line, level)
  Metrics(id, MetricsSample)
  DeviceAlert(kind, detail)
  OperationFinished(op_id, result)
```

**InstanceState** is one enum: `Stopped, Installing, Starting, Running, Stopping, Crashed, Restarting, Updating`. Backup and other operations are overlays tracked by `op_id`, not states.

## 4. Core modules

Each module has one responsibility and a clear done-condition.

### 4.1 Catalog and providers

A `Provider` trait per server type: `versions()`, `resolve(version, build) -> Artifact`, `install(artifact, dir)`. `Artifact` carries URL, hash, hash algorithm, size, and the minimum Java version.

Initial providers (**verify each endpoint against live docs before coding**):

| Type | Source |
|---|---|
| Vanilla | Mojang version manifest v2 (`piston-meta.mojang.com`), per-version JSON for the server download and SHA1 |
| Paper, Folia | PaperMC Fill API (v3) |
| Purpur | Purpur API (`api.purpurmc.org`) |
| Fabric | Fabric meta API (`meta.fabricmc.net`), server launcher JAR |
| Quilt | Quilt meta API |
| Forge | Forge promotions JSON and Maven; run the installer in `--installServer` mode |
| NeoForge | NeoForged Maven metadata; same installer approach |

Out of scope for now: Spigot (needs BuildTools), Pufferfish, CurseForge (needs an API key).

Rules:
- **Verify the published hash of every download.** Abort and delete on mismatch.
- HTTPS only, from a host allowlist, with redirects limited to the allowlist.
- Cache catalogs with a TTL, and work offline from cache when the network is down.
- Version metadata and the Java requirement come from the data source, never from a hardcoded table. For vanilla, read `javaVersion.majorVersion` from the version JSON. For others, derive it from the Minecraft version they target.

Done when: every listed type can install a chosen version into an empty instance directory and reach a startable state.

### 4.2 Java runtime manager

Lodestone builds and hosts no JDKs. It consumes runtimes that other projects already build and maintain, selected from the runtime catalog (section 7).

**Field evidence (how existing Android server apps do it)**
- *MiniCraft* (open source): downloads a bionic OpenJDK 21 "built from Termux packages" on first use into app-private storage (about 250 MB), targets API 34, and starts the JVM via a small native launcher run from `nativeLibraryDir`, which Android treats as executable.
- *MC Host* (open source): bundles OpenJDK 17, 21, and 25 inside the APK, targets API 28 so it can exec binaries from app storage, and picks the Java version per Minecraft version.
- *PojavLauncher lineage*: `dlopen`s the JDK's `libjli.so` from app storage and calls `JLI_Launch` in-process, inside a dedicated `:game` process with a `specialUse` foreground service.
- MC Host supports Minecraft 1.17 and newer only, so it never needed Java 8. Lodestone launches with the same scope.

Conclusion: reusing Termux-built JDKs is proven in practice, and `targetSdk 28` is avoidable. Lodestone downloads runtimes on demand (small APK), not bundled.

**Source order per Java major**

1. **Termux package repository (primary).** Termux publishes headless OpenJDK `.deb` packages for aarch64, arm, i686, and x86_64, rebuilt regularly, on public mirrors. Confirmed present: `openjdk-17` and `openjdk-21`. Java 25 is used by Termux server scripts, so check the live index. Use the headless package, not the `-x` (X11) one.
2. **Community CI builds on GitHub Releases (fallback).** Projects in the PojavLauncher lineage publish Android OpenJDK builds (for example the FCL-Team Android-OpenJDK-Build project covers Java 8, 11, 17, 21, 25; other forks publish individual majors such as 25). Used for Java 8, and for any major Termux does not carry. The original PojavLauncher build repository is archived, so treat these forks as replaceable and never hardcode one.
3. **User-supplied runtime.** The user can import a bionic JDK archive from storage. Always available as a last resort.

**Provisioning from the Termux repository** (no root, no Termux app, no patching)

1. Fetch the repository index (`Release`/`Packages`) and verify its signature against embedded Termux public keys. Verify the key source against Termux's keyring before coding.
2. Find `openjdk-<major>` in `Packages`, and resolve its `Depends` transitively from the same index. **Always include `ca-certificates-java`**, which supplies the Java truststore. Skip tooling-only packages (shells, apt). Confirm the real required set empirically.
3. Download each `.deb`, verify the SHA-256 listed in the index, extract the `.deb` (ar archive containing `data.tar.*`) and place the files under Lodestone's runtimes directory, dropping Termux's absolute prefix.
4. Launch with `JAVA_HOME` set to the extracted JDK and `LD_LIBRARY_PATH` including the extracted library directories. This approach (extract a Termux `.deb` and run it with `JAVA_HOME` and `LD_LIBRARY_PATH`) is a documented community technique, so `patchelf` should not be needed. Set `java.io.tmpdir` to the instance's own temp directory.
5. **Gate and smoke-test** before marking the runtime ready:
   - **Page-size gate**: read the device page size. Read the ELF program headers of the extracted native libraries and compare segment alignment. If the device uses 16 KB pages and any library is aligned to less than 16 KB, mark the runtime unusable on this device, report it on the Device screen, and try the next source. Never attempt to launch.
   - Locate `jssecacerts` in the extracted tree, place it in the JDK's `lib/security` directory, and also pass `-Djavax.net.ssl.trustStore=<path>` and `-Djavax.net.ssl.trustStorePassword=changeit` at launch so it never depends on Termux's absolute paths.
   - Run `java -version`, then an outbound TLS connection to the Mojang session endpoint to confirm the truststore works.

**Known risks to resolve during implementation**
- **Truststore (root cause confirmed):** Termux ships the Java CA keystore in a separate package, so an extracted JDK without it has no usable CA list. Handled by step 2 and step 5 above. Alternative if Termux changes its packaging: build a PKCS12 keystore at install time from Android's own CA store (the system certificates directory, or the conscrypt APEX directory on newer versions) and pass it with `-Djavax.net.ssl.trustStore`. This also honors the device's trusted CAs.
- **16 KB page-size devices:** see the gate in step 5. If Termux's builds are not 16 KB aligned, those devices need a different runtime source. Track this as a known limitation until a source provides aligned builds.
- If the dynamic linker cannot find a dependency via `LD_LIBRARY_PATH`, patch the ELF run path using a pure-Rust ELF editor at install time, as a last resort.
- Termux may change package names, layout, or dependencies. Isolate all of this inside one `TermuxRepoSource` implementation so a breakage is a one-file fix.

**General rules**
- Supports multiple majors side by side and reuses them across instances. Never modifies any system Java.
- Reads the required major from the resolved server artifact. If no source can supply it, say so clearly and offer the user-supplied runtime option.
- Always record each runtime's origin, version, and hash, and show them on the Device screen.
- Check for runtime updates periodically (patch releases of the same major), and apply them only when no instance using that runtime is running.

Done when: starting an instance of any supported version provisions a working runtime automatically, passing the smoke test, with no user action, using only third-party-maintained binaries.

### 4.3 Instance manager

- One directory per instance, with a TOML config describing type, version, build, Java major, memory, tuning profile, port, and options.
- Owns server files, world, and `server.properties` writing.
- Import accepts an existing server folder, detects type and version from its files, and adopts it.
- Multiple instances may exist, but only as many may run as the device's resource budget allows (see 4.5). The core refuses and explains when the budget is exceeded.

### 4.4 Supervisor

- Starts the server through `Platform::launch_server` and tracks the returned `ServerHandle`.
- Graceful stop: send `stop` on stdin, wait up to a timeout, then request termination, then kill (on Android, killing the `:server` process).
- **Classify every exit.** The handle reports exit code and signal. Treat SIGKILL (exit 137) as "killed by the system" (the signature of Android's phantom process killer or low-memory killer), not as a server crash. Show the remedy from the Device guides, restart with backoff, and do not count it toward the crash-loop limit unless it repeats rapidly.
- Real crashes: exponential backoff with a crash-loop limit (for example 3 crashes in 10 minutes ends in `Crashed` with a notification and no further restarts).
- Detects "Done" in logs to move `Starting` to `Running`.
- Never auto-accepts the EULA. The user must accept explicitly, then the core writes `eula.txt`.
- The supervisor survives UI death. On Android the foreground service owns it, so the UI can attach and detach freely and receives a state snapshot on reattach.
- Optional scheduled daily restart with an in-game countdown (via console commands) before stopping.

### 4.5 Device-aware tuning

Inputs: `Platform::memory_info`, `cpu_info`, `thermal_status`, `power_status`.

**Device class** by total RAM: Low (under 4 GB), Mid (4 to 8 GB), High (8 GB and up).

**Memory budget** (constants are tunable defaults, benchmark them):
```
jvm_budget = min(0.55 * MemTotal, MemAvailable - 1024 MB)
heap_max   = 0.75 * jvm_budget      // remainder covers metaspace, threads, native
```
Never allocate more than `MemAvailable` at start. Refuse to start with a clear message if the budget is below a workable minimum (about 768 MB heap).

**JVM flags**
- Always: `-Dfile.encoding=UTF-8`, `-XX:+DisableExplicitGC`, headless.
- High and Mid: Aikar-style G1 flags, `Xms = Xmx`, with `AlwaysPreTouch` off on Android (it wastes RAM and time).
- Low: `-XX:+UseSerialGC`, `Xms` set lower than `Xmx`.
- Flags are generated per runtime version, and unsupported flags are dropped by checking the Java major.

**Server config defaults by class** (applied by writing `server.properties` and, for Paper-family, config files):

| Setting | Low | Mid | High |
|---|---|---|---|
| view-distance | 4 | 6 | 8 |
| simulation-distance | 3 | 4 | 6 |
| network-compression-threshold | 256 | 256 | 256 |
| spawn limits and entity activation | reduced | reduced | default-optimized |

Paper-family servers get an optimized baseline for entity and chunk settings. Plain Vanilla gets only `server.properties` changes. Do not write settings a given server type does not read.

**Phone-specific findings from existing apps and tuning guides**
- Cap the heap at about half of device RAM on phones. Example from MC Host: an 8 GB phone runs 3 to 4 GB.
- Do not use `AlwaysPreTouch` on phones.
- View distance 5 or 6, and pre-generate the world (Chunky) to reduce live chunk-generation load.
- For Fabric servers offer a one-tap "phone optimization pack": Lithium, FerriteCore, Krypton, ModernFix, Chunky.
- Prefer Wi-Fi over mobile data for hosting. Tunnels add latency spikes, so show LAN and tunnel addresses separately and tell LAN players to use the LAN one.

**Thermal and power adaptation**
- `Moderate` thermal: warn, and pause optional heavy work (pre-generation, backups).
- `Severe` or above: warn loudly and offer to restart under a lower profile. Never silently kill a running server.
- Power-save mode on: warn that performance will drop.
- Honest capacity estimate in `device_report`. Do not promise player counts the hardware cannot serve.

Done when: first-run on any supported device yields a startable server with a profile appropriate to its class, with no manual tuning.

### 4.6 Monitoring

Sources:
- **Process**: CPU and resident memory from `/proc/<pid>` (Linux-based platforms).
- **Server**: TPS and MSPT through RCON on Paper-family (`tps`, `mspt`) or Spark if installed. Vanilla 1.20.3+ uses `tick query`. If a metric is unavailable, report it as unavailable. Do not fake it.
- **Players**: `list` via RCON, or the Query protocol.
- **Device**: RAM, temperature, thermal status, battery, charging state, via `Platform`.

A `MetricsSample` is emitted every few seconds while running. The core keeps a bounded in-memory ring buffer (recent history for charts) and exposes the latest sample for surfaces.

RCON is enabled with a random password and bound to localhost only. The password is stored in the instance config and never shown by default.

### 4.7 Backups

- Sequence: `save-off`, `save-all flush`, wait for confirmation, archive the world as tar+zstd, `save-on`. If the server is stopped, skip the save commands.
- Rotation by count and by age. Pre-update backup is automatic.
- Restore stops the server, swaps the world, and restarts if it was running.
- Export copies an archive to the user-visible location via `Platform::export_dir_handle`.
- Backups pause under thermal `Moderate` or higher (see 4.5), and resume after.

### 4.8 Content: mods, plugins, modpacks

- Source: **Modrinth API v2**. Supports search, version resolution by game version and loader, dependency resolution, and update checks by file hash.
- Install into the correct folder for the server type (`mods` vs `plugins`).
- Warn on incompatible loader or game version before installing.
- Keep a lock file per instance so installs are reproducible and rollback is possible.
- Modpacks: install `.mrpack` per the Modrinth modpack format (`modrinth.index.json`, verify hashes, apply overrides).

Done when: a mod or plugin can be searched, installed, updated, and rolled back without leaving the app.

### 4.9 Scheduler

Cron-style tasks per instance: restart, backup, console command, announcement. Runs inside the core while the keep-alive is active. Missed tasks after downtime are skipped, not replayed, unless flagged otherwise.

### 4.10 Console, players, diagnostics

- Console: full scrollback in a bounded buffer, log-level parsing, filtering, and command history held in the core.
- Player management edits `whitelist.json`, `ops.json`, and bans, and uses RCON when the server is running.
- Crash analysis: match the log tail against known patterns (out of memory, port in use, wrong Java version, mod conflict, corrupt chunk) and return a likely cause and suggested fix.

### 4.11 Public access

- LAN address (local IP and port) is always shown.
- **Tunnel**: embed `playit-agent-core` (BSD-2-Clause Rust crate) in the core. Lodestone claims an agent for the install, creates a Minecraft Java tunnel to the local server port, and shows the public address. No external binary is executed. Build it for `aarch64-linux-android` in CI before committing. **Fallback, proven by MiniCraft:** ship the official `playitd` daemon as an arm64 native library in the APK's native library directory, run it, and control it over a local socket. Use whichever matches the playit service's current agent protocol.
- Optional custom domain: compute and, with a user-supplied Cloudflare API token, create the SRV record. Store tokens in private storage and send them only to the provider's API.
- Optional Bedrock crossplay: one switch installs Geyser and Floodgate for the instance's server type and version, and optionally ViaVersion and ViaBackwards for version flexibility.
- Always nudge the user to enable the whitelist when public access is turned on. Offline-mode (cracked) servers require the whitelist.

### 4.12 Sleep when empty, wake on join

- A small proxy in the core owns the public port. While the server is asleep it answers status pings with a configurable "sleeping" message and, on a join attempt, starts the server and holds or kicks the client with a "starting, retry shortly" message.
- Sleeps the server after N minutes with no players. The address stays the same.
- Forward real client IPs using the PROXY protocol header where the server supports it.
- Reference implementations: the `lazymc` project (GPL-3.0) and the sleep feature in MC Host. Reimplement from the protocol rather than copying GPL code, unless the project's license is chosen to allow it.

### 4.13 Remote API

- Optional HTTP + WebSocket API exposing the same commands and event stream as section 3.3. Off by default.
- Auth by PIN or token with attempt lockout. Bound to the LAN only unless the user opts in. Never exposed through the public tunnel.
- Enables a browser panel and lets a future desktop Lodestone manage a phone-hosted server.

## 5. Android platform behavior

This is the Android adapter's responsibility, using the interface in 3.2.

**Server hosting**
- `launch_server` starts a bound/started foreground service declared with `android:process=":server"`. Inside that process, a small native shim `dlopen`s the extracted JDK's launcher library, redirects stdin/stdout/stderr to pipes the core reads and writes, and calls `JLI_Launch` with the generated arguments. Set `LD_LIBRARY_PATH` and `JAVA_HOME` for the shim before launching.
- Stopping the server means the JVM exits, which ends the `:server` process. The main process observes the death and reports the exit. Never try to reuse a process for a second JVM start, since the JVM cannot be created twice in one process.
- Fallback launcher: if in-process loading proves unstable on a device, use a launcher executable placed in the APK's native library directory (set `android:extractNativeLibs="true"` so it is extracted and executable) that `execve`s the JVM as a child process. Expect the phantom process killer to apply in that mode.
- **Residual device tests (acceptance gates):** (1) multi-hour server soak test in the in-process JVM, (2) no SIGKILL on the `:server` process with the screen off on Android 14, 15, and 16, (3) JIT works and TPS is stable, (4) a 16 KB-page device or emulator image. Failures route to the `execve` fallback or to the page-size gate.

**Keep-alive**
- Foreground service of type `specialUse`, with a persistent notification, a partial wake lock, and a Wi-Fi lock, held only while at least one instance is running.
- A boot receiver or alarm re-arms the service after reboot for instances marked auto-start.
- **Notification** is the primary live surface: status, TPS, RAM, players, temperature, with Stop and Restart actions.

**Background exemptions and OEM behavior**
- Guide the user to exempt the app from battery optimization, and detect and report the state.
- OEM skins (for example MIUI/HyperOS) add their own autostart permission that must be enabled separately. Detect the OEM and show the matching steps.
- **Phantom process killer (Android 12+)**: kills phantom processes beyond a system-wide limit of 32 (all apps combined) and processes using excessive CPU. Mitigation order: 1) the `:server` process design above, 2) on Android 14+ the Developer options toggle "Disable child process restrictions" (note it reverts if Developer options is turned off), 3) on Android 12 and 13 the `settings_enable_monitor_phantom_procs` setting via on-device wireless debugging. This setting does not stop the low-memory killer.
- Guides live in the Device screen and are triggered by the supervisor's "killed by the system" classification (4.4).

**Surfaces**: a Compose dashboard, a Glance home-screen widget (status plus start/stop), and a Quick Settings tile. All read the core's latest state and compute nothing.

**Storage**: instances and runtimes live in the app's private storage. Backup export and import use the Storage Access Framework. Allow importing a zipped server folder, and exporting a full-server backup so a server can move between a phone and a PC.

**Networking**: show the local IP and port. External access uses the tunnel (4.11). Direct port forwarding is usually impossible on mobile data because of carrier NAT.

**Process priority**: no root assumed.

Optional and rooted-only (separate, off by default, never required): CPU governor and core pinning, `oom_score_adj` adjustment.

## 6. UI contract (native shell)

The UI implements these screens and calls only the Core interface in 3.3. The list is deliberately platform-neutral so the same design maps to WinUI and SwiftUI.

- **Instances**: list, create (type, version, memory profile), import, delete.
- **Install wizard**: type, version, build, profile, EULA. Shows progress events.
- **Dashboard**: state, live TPS, MSPT, RAM, CPU, players, device temperature, with short history charts.
- **Console**: live output, filter, input with history.
- **Players**: online list, whitelist, ops, bans.
- **Content**: search, installed, updates, modpack install.
- **Backups**: list, create, restore, export, schedule.
- **Config**: validated settings editor with tuning profile selector.
- **Access**: LAN and tunnel addresses, sleep-when-empty settings, Bedrock crossplay switch, remote API PIN.
- **Device**: device report, exemption status, thermal state, setup guides.

Rule: if a screen needs a value the core does not provide, add it to the core, never compute it in the UI.

## 7. Runtime catalog

A small JSON file describes where runtimes come from. It is the only place source URLs live.

- Hosted as a plain file in the Lodestone repository and fetched at runtime. A copy is embedded in the app as a fallback.
- **Changing a source never requires an app release.** If a community fork disappears, edit the JSON.
- Each entry: Java major, OS/arch, ordered list of sources. A source has a type (`termux-repo`, `github-release`, `url`), its location, and for non-repository sources a SHA-256 pin.
- `termux-repo` sources need no pin because the repository index is signature-verified (section 4.2).
- Future platforms add entries for their OS. For example Windows and macOS can use Eclipse Temurin archives, so the Java manager logic stays identical and only the catalog changes.

Maintenance cost is limited to editing this file occasionally. There is no build pipeline, no JDK source tree, and no runtime hosting.

## 8. Porting contract

To add a platform, implement exactly:

1. The `Platform` interface (3.2) in the platform language, using generated bindings.
2. The screens in section 6 in the native UI framework.
3. A background host that keeps the core and supervisor alive when the UI is closed:
   - Android: foreground service (done in this scope).
   - Windows: a background process or Windows Service with a tray icon.
   - macOS: a launchd agent with a menu bar item.
4. Runtime catalog entries for the platform (section 7), and the matching `Platform::runtime_sources` implementation.

No change to modules in section 4 is permitted for a port. If a port seems to need one, the module has leaked platform knowledge and must be fixed first.

## 9. Constraints and non-goals

- No root requirement. Root features are optional extras only.
- No Google Play distribution.
- Do not depend on `targetSdk 28` or on `execve` of files in app storage.
- Never launch a runtime that fails the page-size gate or the truststore smoke test.
- No telemetry or analytics.
- Never run an unverified download.
- Never silently kill or restart a running server except via the crash policy in 4.4.
- No promises about player capacity. Report what the device can honestly support.
- Out of scope for now: native Bedrock Edition servers (crossplay via Geyser is in scope), Spigot, CurseForge, proxies (Velocity, BungeeCord), other tunnel providers, and non-Android platforms.

## 10. Acceptance criteria

The build is correct when, on a mid-range arm64 Android device:

1. A user with no prior setup installs the APK, picks Paper and a version, accepts the EULA, and has a running server with no terminal and no manual Java or tuning.
2. Closing the app and removing it from recents leaves the server running, and the notification still shows live status.
3. Reopening the app reattaches to the running server with current console history and state.
4. The server survives a screen-off period of at least 30 minutes with the documented exemptions applied.
5. A crash triggers the backoff policy, and a crash loop stops with a notification.
6. A backup taken while running restores into a world that loads correctly.
7. Every download is hash-verified, and a corrupted download is rejected. Runtime provisioning works with no Lodestone-built binaries, and online-mode player authentication works (truststore smoke test passes).
8. A Fabric instance installs a Modrinth mod, and the mod loads.
9. The core compiles and its test suite runs on a desktop host with the platform adapter mocked, proving it has no Android dependencies.
10. The app targets a current API level and still starts the JVM (no `execve` from app storage).
11. A SIGKILL of the server process is reported as "killed by the system" with the right guide, and is not counted as a crash.
12. With the server asleep, a client ping shows the sleeping message, and a join attempt starts the server and lets the client connect on retry.
13. A tunnel address from the embedded agent lets a client outside the LAN connect.
14. A 16 KB-page device either runs the server or shows a clear "runtime not compatible with this device" message. It never crashes on `dlopen`.
15. The remote API rejects wrong PINs, locks out after repeated failures, and is unreachable through the tunnel.
