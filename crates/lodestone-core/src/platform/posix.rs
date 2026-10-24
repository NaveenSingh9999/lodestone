//! Headless POSIX execution surface: the real process-launch adapter used by
//! the CLI/Termux build and E2E tests. It implements the `Platform` trait's
//! process bridge (`launch_server`) with actual fork/exec semantics: piped
//! stdio bridged over channels, process-group discipline, SIGKILL group kill,
//! and signal-faithful exit classification via the wait status.
//!
//! Device-integration callbacks that exist only for the Android runtime
//! (keepalive, boot-on-startup, background exemptions, notifications) are
//! explicit surfaces here — no-ops in a headless build — and the Android
//! adapter MUST provide real implementations (spec, platform adapter).

use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use tokio::sync::{mpsc, oneshot};

use crate::platform::{
    CpuInfo, ExemptionState, ExitInfo, LaunchSpec, MemInfo, NetClass, NetInfo, OutputLine,
    PowerStatus, ServerHandle, StatusSummary, Thermal, ThermalLevel, UserNotification,
};
use crate::runtime::RuntimeSource;

pub struct PosixPlatform {
    data_dir: PathBuf,
}

impl PosixPlatform {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        PosixPlatform {
            data_dir: data_dir.into(),
        }
    }
}

impl std::fmt::Debug for PosixPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PosixPlatform(data={})", self.data_dir.display())
    }
}

fn parse_proc_kb(line: &str) -> Option<u64> {
    line.split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<u64>().ok())
        .map(|kb| kb * 1024)
}

fn build_command(spec: &LaunchSpec) -> Result<Command, String> {
    let java = spec.java_home.join("bin/java");
    if !java.exists() {
        return Err(format!("java binary missing: {}", java.display()));
    }
    std::fs::create_dir_all(&spec.work_dir)
        .map_err(|e| format!("mkdir {}: {e}", spec.work_dir.display()))?;
    let jar = if spec.jar.is_absolute() {
        spec.jar.clone()
    } else {
        spec.work_dir.join(&spec.jar)
    };
    if let Some(parent) = jar.parent() {
        if !parent.starts_with(&spec.work_dir) {
            return Err(format!(
                "server jar {} is outside the instance directory {}",
                jar.display(),
                spec.work_dir.display()
            ));
        }
    }

    let mut command = Command::new(&java);
    command
        .current_dir(&spec.work_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // dedicated process group so no descendant can outlive a stop
        .process_group(0);
    command.arg(jar);

    let mut env: HashMap<String, String> = spec.env.clone();
    env.entry("JAVA_HOME".to_string())
        .or_insert_with(|| spec.java_home.to_string_lossy().into_owned());
    for (k, v) in env {
        command.env(k, v);
    }
    command.args(&spec.args);
    Ok(command)
}

fn stdout_bridge<R: std::io::Read + Send + 'static>(
    stream: Option<R>,
    tx: mpsc::UnboundedSender<OutputLine>,
    stderr: bool,
) {
    let mut stream = match stream {
        Some(s) => s,
        None => return,
    };
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut carry: Vec<u8> = Vec::new();
        loop {
            match std::io::Read::read(&mut stream, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    carry.extend_from_slice(&buf[..n]);
                    while let Some(pos) = carry.iter().position(|c| *c == b'\n') {
                        let line: Vec<u8> = carry.drain(..=pos).collect();
                        let text = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                        let _ = tx.send(OutputLine { line: text, stderr });
                    }
                }
                Err(_) => break,
            }
        }
        if !carry.is_empty() {
            let text = String::from_utf8_lossy(&carry).into_owned();
            let _ = tx.send(OutputLine { line: text, stderr });
        }
    });
}

fn exit_bridge(child: &mut std::process::Child, exit_tx: oneshot::Sender<ExitInfo>) {
    let status = child.wait();
    let info = match status {
        Ok(status) => {
            use std::os::unix::process::ExitStatusExt;
            ExitInfo {
                code: status.code(),
                signal: status.signal().map(|s| s as u8),
            }
        }
        Err(e) => {
            let _ = e;
            ExitInfo {
                code: Some(-1),
                signal: None,
            }
        }
    };
    let _ = exit_tx.send(info);
}

impl crate::platform::Platform for PosixPlatform {
    fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn export_dir_handle(&self) -> Option<PathBuf> {
        Some(self.data_dir.join("exports"))
    }

    fn begin_keepalive(&self, _reason: &str) {
        // headless: no Doze semantics; Android adapter overrides
    }

    fn end_keepalive(&self) {}

    fn request_background_exemptions(&self) -> ExemptionState {
        // Linux is always exempt; adapter on Android must interop with the OS.
        ExemptionState::NotExempt
    }

    fn run_on_boot(&self, _enabled: bool) {}

    fn memory_info(&self) -> MemInfo {
        let mut info = MemInfo {
            total_bytes: 0,
            available_bytes: 0,
            low_memory: true,
        };
        if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
            for line in text.lines() {
                if line.starts_with("MemTotal:") {
                    info.total_bytes = parse_proc_kb(line).unwrap_or(0);
                } else if line.starts_with("MemAvailable:") {
                    info.available_bytes = parse_proc_kb(line).unwrap_or(0);
                }
            }
        }
        info.low_memory = info.available_bytes < 768 * 1024 * 1024;
        info
    }

    fn cpu_info(&self) -> CpuInfo {
        let mut core_count = 0u32;
        let mut max_freq = 0u32;
        if let Ok(entries) = std::fs::read_dir("/sys/devices/system/cpu") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.len() < 4 || !name.starts_with("cpu") {
                    continue;
                }
                let suffix = &name[3..];
                if !suffix.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                core_count += 1;
                let freq_path = entry.path().join("cpufreq/cpuinfo_max_freq");
                if let Ok(text) = std::fs::read_to_string(&freq_path) {
                    if let Ok(khz) = text.trim().parse::<u32>() {
                        max_freq = max_freq.max(khz / 1000);
                    }
                }
            }
        }
        CpuInfo {
            core_count,
            max_freq_mhz: max_freq,
        }
    }

    fn thermal_status(&self) -> Thermal {
        let mut hottest: Option<f32> = None;
        if let Ok(zones) = std::fs::read_dir("/sys/class/thermal") {
            for entry in zones.flatten() {
                if let Ok(text) = std::fs::read_to_string(entry.path().join("temp")) {
                    if let Ok(millic) = text.trim().parse::<i64>() {
                        let c = millic as f32 / 1000.0;
                        hottest = Some(hottest.map_or(c, |h: f32| h.max(c)));
                    }
                }
            }
        }
        let temp = hottest.unwrap_or(0.0);
        let level = classify_thermal(temp);
        Thermal {
            level,
            temperature_c: hottest,
        }
    }

    fn power_status(&self) -> PowerStatus {
        let capacity = std::fs::read_to_string("/sys/class/power_supply/battery/capacity")
            .ok()
            .and_then(|t| t.trim().parse().ok());
        let charging_status = std::fs::read_to_string("/sys/class/power_supply/battery/status")
            .map(|t| t.trim().to_lowercase())
            .unwrap_or_default();
        PowerStatus {
            battery_percent: capacity,
            charging: charging_status == "charging" || charging_status == "full",
            power_save: false,
        }
    }

    fn network_info(&self) -> NetInfo {
        NetInfo {
            addresses: vec![],
            class: NetClass::Unknown,
        }
    }

    fn show_status(&self, summary: &StatusSummary) {
        eprintln!(
            "[lodestone] running={:?} starting={:?} {}",
            summary.running_instances, summary.starting_instances, summary.headline
        );
    }

    fn notify(&self, event: &UserNotification) {
        eprintln!("[lodestone/notify] {}: {}", event.title, event.body);
    }

    fn launch_server(&self, spec: LaunchSpec) -> ServerHandle {
        let (stdin_tx, stdin_rx) = mpsc::unbounded_channel::<String>();
        let (out_tx, out_rx) = mpsc::unbounded_channel::<OutputLine>();
        let (exit_tx, exit_rx) = oneshot::channel::<ExitInfo>();

        match build_command(&spec) {
            Ok(mut command) => match command.spawn() {
                Ok(mut child) => {
                    let pid: i32 = child.id() as i32;
                    let pid_shared = Arc::new(AtomicI32::new(pid));

                    if let Some(stdin) = child.stdin.take() {
                        let mut rx = stdin_rx;
                        std::thread::spawn(move || {
                            let mut writer = std::io::BufWriter::new(stdin);
                            loop {
                                match rx.blocking_recv() {
                                    Some(line) => {
                                        use std::io::Write;
                                        let _ = writer.write_all(line.as_bytes());
                                        let _ = writer.write_all(b"\n");
                                    }
                                    None => break,
                                }
                            }
                        });
                    }

                    stdout_bridge(child.stdout.take(), out_tx.clone(), false);
                    stdout_bridge(child.stderr.take(), out_tx, false);

                    std::thread::spawn(move || exit_bridge(&mut child, exit_tx));

                    let killer_pid = Arc::clone(&pid_shared);
                    let kill = Box::new(move || {
                        unsafe {
                            // two shots: whole process group, then itself
                            libc::kill(-killer_pid.load(Ordering::SeqCst), libc::SIGKILL);
                            libc::kill(killer_pid.load(Ordering::SeqCst), libc::SIGKILL);
                        }
                    });
                    ServerHandle::new(stdin_tx, out_rx, exit_rx, kill)
                }
                Err(e) => {
                    let _ = out_tx.send(OutputLine {
                        line: format!("[launch] spawn failed: {e}"),
                        stderr: true,
                    });
                    let _ = exit_tx.send(ExitInfo {
                        code: Some(1),
                        signal: None,
                    });
                    ServerHandle::new(stdin_tx, out_rx, exit_rx, Box::new(|| {}))
                }
            },
            Err(reason) => {
                let _ = out_tx.send(OutputLine {
                    line: format!("[launch] {reason}"),
                    stderr: true,
                });
                let _ = exit_tx.send(ExitInfo {
                    code: Some(1),
                    signal: None,
                });
                ServerHandle::new(stdin_tx, out_rx, exit_rx, Box::new(|| {}))
            }
        }
    }

    fn runtime_sources(&self, java_major: u32) -> Vec<RuntimeSource> {
        let _ = java_major;
        vec![RuntimeSource::TermuxRepo { mirror: None }]
    }
}

fn classify_thermal(c: f32) -> ThermalLevel {
    use ThermalLevel::*;
    if c >= 60.0 {
        Critical
    } else if c >= 55.0 {
        Severe
    } else if c >= 45.0 {
        Moderate
    } else if c >= 38.0 {
        Light
    } else {
        None
    }
}
