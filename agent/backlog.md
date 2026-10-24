# Lodestone backlog

1. [M1] Runtime provisioning — Termux repo `.deb` pipeline: index fetch + signature verify, dep resolution incl. ca-certificates-java, truststore placement, ELF gate on every lib, `java -version` + TLS smoke test (spec 4.2)
2. [M2] PaperMC Fill v3 provider + Folia/Purpur stubs behind same Provider trait (spec 4.1)
3. [M3] Supervisor live loop: stdin/stdout pump, Done detection, backoff restarts, crash windows (spec 4.4)
4. [M4] UniFFI FFI surface (section 3.3 commands/events) + Kotlin adapter skeleton
5. [M5] Android `:server` service process + dlopen/JLI_Launch shim (spec 5; soak test acceptance gate)
6. Sleep-when-empty proxy, tunnel verdict, remote API — after M5 (spec 4.11-4.13)
