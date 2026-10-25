//! Android platform adapter.
//!
//! Hosts the JVM inside the `:server` service process by `dlopen`ing the
//! extracted JDK's `libjli.so` and calling `JLI_Launch` (spec §2 decision,
//! spec §5 "Server hosting", acceptance criterion #10). stdio is redirected
//! to pipes so the core can read/write console output through the
//! `ServerHandle`, exactly like the POSIX fork/exec path.
//!
//! This module is gated to `target_os = "android"`. On other targets
//! `PosixPlatform` (fork/exec) is used — that is also the documented Android
//! fallback if in-process loading proves unstable on a device (spec §5).
//!
//! The JLI_Launch path has NOT been device-tested (no Android SDK/emulator in
//! this environment) and is listed as a residual acceptance gate in spec §5 /
//! §10: it needs a multi-hour soak test, a screen-off SIGKILL check, and a
//! 16 KB-page device test before shipping.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::FromRawFd;
use std::path::{Path, PathBuf};
use std::thread;

use libc::{O_CLOEXEC, RTLD_NOW, close, dlclose, dlerror, dlopen, dlsym, dup, dup2, pipe2};
use tokio::sync::{mpsc, oneshot};

use crate::platform::{
    CpuInfo, ExemptionState, ExitInfo, LaunchSpec, MemInfo, NetClass, NetInfo, Platform,
    PowerStatus, RuntimeSource, ServerHandle, StatusSummary, Thermal, ThermalLevel,
    UserNotification,
};

/// `jint JLI_Launch(int argc, const char *const *argv, ...)`.
///
/// The exact OpenJDK signature varies by platform (some declarations carry a
/// trailing `javaMainClass`/`javaArgs`/flags variadic). Projects in the
/// PojavLauncher lineage call it with just `(argc, argv)` and pass the main
/// class/jar through `argv` itself, which is what we do here.
type JliLaunchFn = unsafe extern "C" fn(c_int, *const *const c_char) -> c_int;

pub struct AndroidPlatform {
    data_dir: PathBuf,
}

impl AndroidPlatform {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }
}

impl std::fmt::Debug for AndroidPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AndroidPlatform(data={})", self.data_dir.display())
    }
}

impl Platform for AndroidPlatform {
    fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn export_dir_handle(&self) -> Option<PathBuf> {
        None // Storage Access Framework is handled by the Kotlin adapter.
    }

    fn begin_keepalive(&self, _reason: &str) {
        // Foreground service + wake/Wi-Fi locks are owned by the Kotlin
        // adapter; this Rust surface is a no-op until UniFFI bindings exist.
    }

    fn end_keepalive(&self) {}

    fn request_background_exemptions(&self) -> ExemptionState {
        ExemptionState::Unknown
    }

    fn run_on_boot(&self, _enabled: bool) {}

    fn memory_info(&self) -> MemInfo {
        MemInfo {
            total_bytes: 0,
            available_bytes: 0,
            low_memory: false,
        } // stub — real values come from the Kotlin adapter / /proc
    }

    fn cpu_info(&self) -> CpuInfo {
        CpuInfo {
            core_count: 0,
            max_freq_mhz: 0,
        }
    }

    fn thermal_status(&self) -> Thermal {
        Thermal {
            level: ThermalLevel::None,
            temperature_c: None,
        }
    }

    fn power_status(&self) -> PowerStatus {
        PowerStatus {
            battery_percent: None,
            charging: false,
            power_save: false,
        }
    }

    fn network_info(&self) -> NetInfo {
        NetInfo {
            addresses: Vec::new(),
            class: NetClass::Unknown,
        }
    }

    fn show_status(&self, _summary: &StatusSummary) {
        // Notification UI is owned by the Kotlin adapter.
    }

    fn notify(&self, _event: &UserNotification) {
        // Notification UI is owned by the Kotlin adapter.
    }

    fn launch_server(&self, spec: LaunchSpec) -> ServerHandle {
        launch_via_jli(&spec)
    }

    fn runtime_sources(&self, _java_major: u32) -> Vec<RuntimeSource> {
        vec![RuntimeSource::TermuxRepo { mirror: None }]
    }
}

/// Locate `libjli.so` inside a provisioned JDK home.
///
/// Tries the Android layout first (`lib/aarch64/libjli.so`), then the generic
/// OpenJDK layout (`lib/jli/libjli.so`). Returns the first path that exists.
fn find_libjli(java_home: &Path) -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = vec![
        java_home.join("lib/aarch64/libjli.so"),
        java_home.join("lib/jli/libjli.so"),
        java_home.join("lib/libjli.so"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// Build the `JLI_Launch` argument vector from a `LaunchSpec`.
///
/// Mirrors what `java -jar <jar> [args...]` would pass to the launcher:
/// `argv[0]` is the launcher name, then `-jar`, the jar path, then any extra
/// args (JVM flags and server arguments should already be folded into
/// `spec.args` by the tuning/config layer).
fn build_jli_argv(spec: &LaunchSpec) -> Result<Vec<CString>, String> {
    let jar = if spec.jar.is_absolute() {
        spec.jar.clone()
    } else {
        spec.work_dir.join(&spec.jar)
    };
    if let Some(parent) = jar.parent()
        && !parent.starts_with(&spec.work_dir)
    {
        return Err(format!(
            "server jar {} is outside the instance directory {}",
            jar.display(),
            spec.work_dir.display()
        ));
    }

    let mut argv: Vec<CString> = Vec::new();
    argv.push(CString::new("java").map_err(|e| format!("argv0: {e}"))?);
    argv.push(CString::new("-jar").map_err(|e| format!("arg: {e}"))?);
    argv.push(CString::new(jar.to_string_lossy().as_ref()).map_err(|e| format!("jar path: {e}"))?);
    for arg in &spec.args {
        argv.push(CString::new(arg.as_str()).map_err(|e| format!("arg: {e}"))?);
    }
    Ok(argv)
}

/// Launch a server inside the current process via `dlopen(libjli.so)` +
/// `JLI_Launch`.
///
/// stdio is redirected to pipes (`dup2` on fds 0/1/2) so the core can bridge
/// console output over the `ServerHandle` channels. `JLI_Launch` runs on a
/// dedicated thread and blocks until the JVM shuts down; its exit code is
/// delivered through the handle's exit receiver.
fn launch_via_jli(spec: &LaunchSpec) -> ServerHandle {
    let result = (|| -> Result<ServerHandle, String> {
        // Validate the spec before touching native resources.
        crate::platform::validate_launch_spec(spec)?;

        // Channels live inside the closure so they are moved into it once and
        // can be used by both the spawn threads and the error/fallback paths.
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
        let (out_tx, out_rx) = mpsc::unbounded_channel::<crate::platform::OutputLine>();
        let (exit_tx, exit_rx) = oneshot::channel::<ExitInfo>();

        let java_home = &spec.java_home;
        let mut ld_paths: Vec<String> = Vec::new();
        ld_paths.push(java_home.join("lib").to_string_lossy().into_owned());
        ld_paths.push(java_home.join("lib/aarch64").to_string_lossy().into_owned());
        ld_paths.push(java_home.join("lib/jli").to_string_lossy().into_owned());
        if let Ok(existing) = std::env::var("LD_LIBRARY_PATH") {
            ld_paths.push(existing);
        }
        // Setting process-wide env is unsafe in Rust 1.97 but required here
        // to make libjvm.so discoverable by dlopen; safe within the :server
        // process which owns its environment.
        unsafe {
            std::env::set_var("LD_LIBRARY_PATH", ld_paths.join(":"));
            std::env::set_var("JAVA_HOME", java_home.to_string_lossy().as_ref());
        }
        let libjli = find_libjli(java_home)
            .ok_or_else(|| format!("libjli.so not found in {}", java_home.display()))?;

        let handle = unsafe {
            dlopen(
                libjli.as_os_str().as_bytes().as_ptr() as *const c_char,
                RTLD_NOW,
            )
        };
        if handle.is_null() {
            let msg = unsafe { CStr::from_ptr(dlerror()) }
                .to_string_lossy()
                .into_owned();
            return Err(format!("dlopen {}: {}", libjli.display(), msg));
        }

        let jli_launch = unsafe { dlsym(handle, c"JLI_Launch".as_ptr()) };
        if jli_launch.is_null() {
            unsafe { dlclose(handle) };
            let msg = unsafe { CStr::from_ptr(dlerror()) }
                .to_string_lossy()
                .into_owned();
            return Err(format!("dlsym JLI_Launch: {msg}"));
        }
        let jli_launch: JliLaunchFn = unsafe { std::mem::transmute(jli_launch) };

        // --- build argv -------------------------------------------------
        let argv_cstr = build_jli_argv(spec)?;
        let argc = argv_cstr.len() as c_int;

        // --- create stdio pipes ----------------------------------------
        let mk_pipe = || -> Result<(i32, i32), String> {
            let mut fds: [c_int; 2] = [0, 0];
            if unsafe { pipe2(fds.as_mut_ptr(), O_CLOEXEC) } != 0 {
                return Err("pipe2 failed".into());
            }
            Ok((fds[0], fds[1]))
        };
        let (stdin_r, stdin_w) = mk_pipe()?; // JVM reads stdin_r, we write stdin_w
        let (stdout_r, stdout_w) = mk_pipe()?; // JVM writes stdout_w, we read stdout_r
        let (stderr_r, stderr_w) = mk_pipe()?; // JVM writes stderr_w, we read stderr_r

        // Redirect process stdio fds 0/1/2 into the pipes.
        if unsafe { dup2(stdin_r, 0) } < 0 {
            return Err("dup2 stdin failed".into());
        }
        if unsafe { dup2(stdout_w, 1) } < 0 {
            return Err("dup2 stdout failed".into());
        }
        if unsafe { dup2(stderr_w, 2) } < 0 {
            return Err("dup2 stderr failed".into());
        }
        // The original pipe fds are now duplicated onto 0/1/2; close the
        // copies we no longer address directly.
        let _ = unsafe { close(stdin_r) };
        let _ = unsafe { close(stdout_w) };
        let _ = unsafe { close(stderr_w) };

        // Duplicate stdin_w so `kill()` can close its own copy independently
        // of the writer thread's fd (EOF on all write ends signals stdin EOF).
        let stdin_w_kill = unsafe { dup(stdin_w) };
        if stdin_w_kill < 0 {
            return Err("dup stdin_w failed".into());
        }

        // --- stdout bridge --------------------------------------------
        let out_tx_stdout = out_tx.clone();
        thread::spawn(move || {
            read_loop(stdout_r, out_tx_stdout, false);
        });
        let out_tx_stderr = out_tx.clone();
        thread::spawn(move || {
            read_loop(stderr_r, out_tx_stderr, true);
        });

        // --- stdin writer ---------------------------------------------
        thread::spawn(move || {
            let mut writer = unsafe { std::fs::File::from_raw_fd(stdin_w) };
            use std::io::Write;
            while let Some(line) = stdin_rx.blocking_recv() {
                let _ = writer.write_all(line.as_bytes());
                let _ = writer.write_all(b"\n");
                let _ = writer.flush();
            }
            // Dropping `writer` closes stdin_w -> JVM sees EOF on stdin.
        });

        // --- JLI_Launch thread ----------------------------------------
        // Cast the dlopen handle to `usize` (Send) so it can cross the thread
        // boundary; `argv_cstr` (Vec<CString>, Send) moves into the thread and
        // the argv pointer array is rebuilt there from the live CStrings.
        let handle_val = handle as usize;
        thread::spawn(move || {
            let _ = argv_cstr; // keep CString allocations alive for pointer validity
            let handle = handle_val as *mut c_void;
            let mut local_ptrs: Vec<*const c_char> = argv_cstr.iter().map(|s| s.as_ptr()).collect();
            local_ptrs.push(std::ptr::null());
            let exit_code = unsafe { jli_launch(argc, local_ptrs.as_ptr()) };
            // Close the dlopen handle once the JVM is done.
            let _ = unsafe { dlclose(handle) };
            let exit_info = match exit_code {
                0 => ExitInfo::clean(),
                137 => ExitInfo {
                    code: Some(137),
                    signal: None,
                },
                code => ExitInfo {
                    code: Some(code),
                    signal: None,
                },
            };
            let _ = exit_tx.send(exit_info);
        });

        let kill = move || {
            // Best-effort: close our copy of the stdin write end. Full process
            // destruction on Android is the Kotlin service's responsibility
            // (spec §5: stopping the JVM ends the `:server` process).
            let _ = unsafe { close(stdin_w_kill) };
        };

        Ok(ServerHandle::new(stdin_tx, out_rx, exit_rx, Box::new(kill)))
    })();

    match result {
        Ok(h) => h,
        Err(_e) => {
            // Launch failed before a handle could be created; return a
            // disconnected handle so the caller can surface the error.
            let (stdin_tx, _stdin_rx) = mpsc::unbounded_channel::<String>();
            let (_out_tx, out_rx) = mpsc::unbounded_channel::<crate::platform::OutputLine>();
            let (_exit_tx, exit_rx) = oneshot::channel::<ExitInfo>();
            ServerHandle::new(stdin_tx, out_rx, exit_rx, Box::new(|| {}))
        }
    }
}

/// Read lines from a pipe fd and forward them as `OutputLine` events.
fn read_loop(fd: c_int, tx: mpsc::UnboundedSender<crate::platform::OutputLine>, stderr: bool) {
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut buf = [0u8; 8192];
    let mut carry: Vec<u8> = Vec::new();
    loop {
        match std::io::Read::read(&mut file, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                carry.extend_from_slice(&buf[..n]);
                while let Some(pos) = carry.iter().position(|c| *c == b'\n') {
                    let line: Vec<u8> = carry.drain(..=pos).collect();
                    let text = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                    let _ = tx.send(crate::platform::OutputLine { line: text, stderr });
                }
            }
            Err(_) => break,
        }
    }
    if !carry.is_empty() {
        let text = String::from_utf8_lossy(&carry).into_owned();
        let _ = tx.send(crate::platform::OutputLine { line: text, stderr });
    }
    // Dropping `file` closes the read end of the pipe.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_jli_argv_keeps_jar_inside_workdir() {
        let spec = LaunchSpec {
            instance_id: "t".into(),
            java_home: "/data/jdk".into(),
            jar: "server.jar".into(),
            args: vec!["nogui".into()],
            env: Default::default(),
            work_dir: "/data/instance".into(),
            rcon_port: None,
            rcon_password: None,
        };
        let argv = build_jli_argv(&spec).unwrap();
        assert_eq!(argv[0].to_str().unwrap(), "java");
        assert_eq!(argv[1].to_str().unwrap(), "-jar");
        assert!(argv[2].to_string_lossy().ends_with("server.jar"));
        assert_eq!(argv[3].to_str().unwrap(), "nogui");
        assert_eq!(argv.len(), 4);
    }

    #[test]
    fn build_jli_argv_rejects_jar_outside_workdir() {
        let spec = LaunchSpec {
            instance_id: "t".into(),
            java_home: "/data/jdk".into(),
            jar: "/etc/passwd".into(),
            args: vec![],
            env: Default::default(),
            work_dir: "/data/instance".into(),
            rcon_port: None,
            rcon_password: None,
        };
        assert!(build_jli_argv(&spec).is_err());
    }

    #[test]
    fn find_libjli_returns_none_when_missing() {
        assert!(find_libjli(Path::new("/nonexistent")).is_none());
    }
}
