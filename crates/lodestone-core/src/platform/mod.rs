use std::collections::HashMap;
use std::path::PathBuf;

use tokio::sync::{mpsc, oneshot};

use crate::runtime::RuntimeSource;

pub mod posix;

#[cfg(target_os = "android")]
pub mod android;
pub use posix::PosixPlatform;

#[derive(Debug, Clone)]
pub struct MemInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub low_memory: bool,
}

#[derive(Debug, Clone)]
pub struct CpuInfo {
    pub core_count: u32,
    pub max_freq_mhz: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalLevel {
    None,
    Light,
    Moderate,
    Severe,
    Critical,
}

impl ThermalLevel {
    pub fn blocks_optional_work(self) -> bool {
        self >= ThermalLevel::Moderate
    }
}

impl PartialOrd for ThermalLevel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ThermalLevel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

#[derive(Debug, Clone)]
pub struct Thermal {
    pub level: ThermalLevel,
    pub temperature_c: Option<f32>,
}

impl Thermal {
    pub fn blocks_optional_work(&self) -> bool {
        self.level.blocks_optional_work()
    }
}

#[derive(Debug, Clone)]
pub struct PowerStatus {
    pub battery_percent: Option<u8>,
    pub charging: bool,
    pub power_save: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetClass {
    Wifi,
    Cellular,
    Hotspot,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct NetInfo {
    pub addresses: Vec<String>,
    pub class: NetClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExemptionState {
    Unknown,
    Exempt,
    NotExempt,
}

#[derive(Debug, Clone)]
pub struct UserNotification {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct StatusSummary {
    pub running_instances: Vec<String>,
    pub starting_instances: Vec<String>,
    pub headline: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitKind {
    Clean,
    Crashed { code: i32 },
    KilledBySystem,
}

impl ExitKind {
    pub fn remedy_hint(&self) -> Option<&'static str> {
        match self {
            ExitKind::KilledBySystem => Some("device.process_killed"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub signal: Option<u8>,
}

impl ExitInfo {
    pub fn clean() -> Self {
        ExitInfo {
            code: Some(0),
            signal: None,
        }
    }

    pub fn kind(&self) -> ExitKind {
        if self.signal == Some(9) {
            return ExitKind::KilledBySystem;
        }
        match self.code {
            Some(0) => ExitKind::Clean,
            Some(137) => ExitKind::KilledBySystem,
            Some(code) => ExitKind::Crashed { code },
            None => ExitKind::Crashed { code: -1 },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LaunchSpec {
    pub instance_id: String,
    pub java_home: PathBuf,
    pub jar: PathBuf,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub work_dir: PathBuf,
    /// Optional RCON server port for metrics collection.
    pub rcon_port: Option<u16>,
    /// Optional RCON password for authentication.
    pub rcon_password: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OutputLine {
    pub line: String,
    pub stderr: bool,
}

pub struct ServerHandle {
    stdin: mpsc::UnboundedSender<String>,
    output: mpsc::UnboundedReceiver<OutputLine>,
    exit: Option<oneshot::Receiver<ExitInfo>>,
    kill: Box<dyn Fn() + Send>,
}

impl ServerHandle {
    pub fn new(
        stdin: mpsc::UnboundedSender<String>,
        output: mpsc::UnboundedReceiver<OutputLine>,
        exit: oneshot::Receiver<ExitInfo>,
        kill: Box<dyn Fn() + Send>,
    ) -> Self {
        ServerHandle {
            stdin,
            output,
            exit: Some(exit),
            kill,
        }
    }

    pub fn send_console(&self, line: impl Into<String>) -> bool {
        self.stdin.send(line.into()).is_ok()
    }

    pub async fn recv_output(&mut self) -> Option<OutputLine> {
        self.output.recv().await
    }

    pub fn take_exit(&mut self) -> Option<oneshot::Receiver<ExitInfo>> {
        self.exit.take()
    }

    pub fn kill(&self) {
        (self.kill)();
    }
}

pub trait Platform: Send + Sync {
    fn data_dir(&self) -> PathBuf;
    fn export_dir_handle(&self) -> Option<PathBuf>;

    fn begin_keepalive(&self, reason: &str);
    fn end_keepalive(&self);
    fn request_background_exemptions(&self) -> ExemptionState;
    fn run_on_boot(&self, enabled: bool);

    fn memory_info(&self) -> MemInfo;
    fn cpu_info(&self) -> CpuInfo;
    fn thermal_status(&self) -> Thermal;
    fn power_status(&self) -> PowerStatus;
    fn network_info(&self) -> NetInfo;

    fn show_status(&self, summary: &StatusSummary);
    fn notify(&self, event: &UserNotification);

    fn launch_server(&self, spec: LaunchSpec) -> ServerHandle;
    fn runtime_sources(&self, java_major: u32) -> Vec<RuntimeSource>;
}

pub fn effective_page_size() -> u32 {
    crate::runtime::device_page_size().unwrap_or(4096)
}

pub fn validate_launch_spec(spec: &LaunchSpec) -> Result<(), String> {
    if spec.instance_id.is_empty() {
        return Err("instance id is empty".into());
    }
    if spec.java_home.as_os_str().is_empty() {
        return Err("java home is empty".into());
    }
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("lode-core-{}", name));
        std::fs::create_dir_all(&base).ok();
        base
    }

    #[test]
    fn kill_classification_follows_spec() {
        assert_eq!(ExitInfo::clean().kind(), ExitKind::Clean);
        assert_eq!(
            ExitInfo {
                code: Some(137),
                signal: None
            }
            .kind(),
            ExitKind::KilledBySystem
        );
        assert_eq!(
            ExitInfo {
                code: None,
                signal: Some(9)
            }
            .kind(),
            ExitKind::KilledBySystem
        );
        assert_eq!(
            ExitInfo {
                code: Some(1),
                signal: None
            }
            .kind(),
            ExitKind::Crashed { code: 1 }
        );
    }

    #[test]
    fn launch_spec_rejects_escape() {
        let work = tmp_dir("spec-ok");
        let ok = LaunchSpec {
            instance_id: "a".into(),
            java_home: work.join("jdk"),
            jar: work.join("server.jar"),
            env: HashMap::new(),
            args: vec![],
            work_dir: work.clone(),
            rcon_port: None,
            rcon_password: None,
        };
        validate_launch_spec(&ok).unwrap();

        let work2 = tmp_dir("spec-other");
        let escape = LaunchSpec {
            instance_id: "a".into(),
            java_home: work2.join("jdk"),
            jar: work2.join("server.jar"),
            env: HashMap::new(),
            args: vec![],
            work_dir: work.clone(),
            rcon_port: None,
            rcon_password: None,
        };
        assert!(validate_launch_spec(&escape).is_err());
    }

    #[test]
    fn thermal_levels_order() {
        assert!(ThermalLevel::Moderate.blocks_optional_work());
        assert!(!ThermalLevel::Light.blocks_optional_work());
        assert!(ThermalLevel::Severe > ThermalLevel::Moderate);
    }
}
