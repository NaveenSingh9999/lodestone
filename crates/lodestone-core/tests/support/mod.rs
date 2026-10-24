#[allow(dead_code)]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tokio::sync::{mpsc, oneshot};

use lodestone_core::error::NetError;
use lodestone_core::net::HttpClient;
use lodestone_core::platform::{
    CpuInfo, ExemptionState, ExitInfo, LaunchSpec, MemInfo, NetClass, NetInfo, OutputLine,
    Platform, PowerStatus, ServerHandle, StatusSummary, Thermal, ThermalLevel, UserNotification,
};
use lodestone_core::runtime::RuntimeSource;

#[allow(dead_code)]
pub struct FixtureHttpClient {
    routes: Mutex<HashMap<String, Vec<u8>>>,
}

#[allow(dead_code)]
impl FixtureHttpClient {
    pub fn new() -> Self {
        FixtureHttpClient {
            routes: Mutex::new(HashMap::new()),
        }
    }

    pub fn route(&self, url: impl Into<String>, body: Vec<u8>) {
        self.routes.lock().unwrap().insert(url.into(), body);
    }

    pub fn route_json(self, url: &str, json: &str) -> Self {
        self.route(url.to_string(), json.as_bytes().to_vec());
        self
    }
}

impl Default for FixtureHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient for FixtureHttpClient {
    async fn get(&self, url: &str) -> Result<Vec<u8>, NetError> {
        match self.routes.lock().unwrap().get(url) {
            Some(v) => Ok(v.clone()),
            None => Err(NetError::Http(format!("no fixture route for {url}"))),
        }
    }

    async fn download(&self, url: &str, dest: &Path) -> Result<u64, NetError> {
        let body = self.get(url).await?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| NetError::Http(format!("mkdir: {e}")))?;
        }
        std::fs::write(dest, &body).map_err(|e| NetError::Http(format!("write: {e}")))?;
        Ok(body.len() as u64)
    }
}

#[allow(dead_code)]
pub struct TestPlatform {
    data_dir: PathBuf,
    pub mem: MemInfo,
    pub cpu: CpuInfo,
    pub thermal: Thermal,
    pub power: PowerStatus,
    pub net: NetInfo,
    pub exemptions: ExemptionState,
    pub notifications: mpsc::UnboundedReceiver<UserNotification>,
    pub statuses: mpsc::UnboundedReceiver<StatusSummary>,
    keepalive_active: Mutex<bool>,
    on_boot: Mutex<bool>,
    notifications_tx: mpsc::UnboundedSender<UserNotification>,
    statuses_tx: mpsc::UnboundedSender<StatusSummary>,
}

#[allow(dead_code)]
impl TestPlatform {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        let (notifications_tx, notifications) = mpsc::unbounded_channel();
        let (statuses_tx, statuses) = mpsc::unbounded_channel();
        TestPlatform {
            data_dir: data_dir.as_ref().to_path_buf(),
            mem: MemInfo {
                total_bytes: 8 * 1024 * 1024 * 1024,
                available_bytes: 5 * 1024 * 1024 * 1024,
                low_memory: false,
            },
            cpu: CpuInfo {
                core_count: 8,
                max_freq_mhz: 2800,
            },
            thermal: Thermal {
                level: ThermalLevel::None,
                temperature_c: None,
            },
            power: PowerStatus {
                battery_percent: Some(80),
                charging: true,
                power_save: false,
            },
            net: NetInfo {
                addresses: vec!["192.168.1.42".into()],
                class: NetClass::Wifi,
            },
            exemptions: ExemptionState::Exempt,
            notifications,
            statuses,
            keepalive_active: Mutex::new(false),
            on_boot: Mutex::new(false),
            notifications_tx,
            statuses_tx,
        }
    }

    pub fn set_thermal(&mut self, level: ThermalLevel) {
        self.thermal = Thermal {
            level,
            temperature_c: None,
        };
    }

    pub fn set_power_save(&mut self, on: bool) {
        self.power.power_save = on;
    }

    pub fn keepalive_active(&self) -> bool {
        *self.keepalive_active.lock().unwrap()
    }

    pub fn on_boot(&self) -> bool {
        *self.on_boot.lock().unwrap()
    }
}

impl Platform for TestPlatform {
    fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn export_dir_handle(&self) -> Option<PathBuf> {
        Some(self.data_dir.join("export"))
    }

    fn begin_keepalive(&self, reason: &str) {
        *self.keepalive_active.lock().unwrap() = true;
        let _ = self.notifications_tx.send(UserNotification {
            title: "keepalive".into(),
            body: reason.into(),
        });
    }

    fn end_keepalive(&self) {
        *self.keepalive_active.lock().unwrap() = false;
    }

    fn request_background_exemptions(&self) -> ExemptionState {
        self.exemptions.clone()
    }

    fn run_on_boot(&self, enabled: bool) {
        *self.on_boot.lock().unwrap() = enabled;
    }

    fn memory_info(&self) -> MemInfo {
        self.mem.clone()
    }

    fn cpu_info(&self) -> CpuInfo {
        self.cpu.clone()
    }

    fn thermal_status(&self) -> Thermal {
        self.thermal.clone()
    }

    fn power_status(&self) -> PowerStatus {
        self.power.clone()
    }

    fn network_info(&self) -> NetInfo {
        self.net.clone()
    }

    fn show_status(&self, summary: &StatusSummary) {
        let _ = self.statuses_tx.send(summary.clone());
    }

    fn notify(&self, event: &UserNotification) {
        let _ = self.notifications_tx.send(event.clone());
    }

    fn launch_server(&self, _spec: LaunchSpec) -> ServerHandle {
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
        let (out_tx, out_rx) = mpsc::unbounded_channel::<OutputLine>();
        let (exit_tx, exit_rx) = oneshot::channel::<ExitInfo>();
        tokio::spawn(async move {
            while let Some(line) = stdin_rx.recv().await {
                match line.as_str() {
                    "stop" => {
                        let _ = out_tx.send(OutputLine {
                            line: "[test] Stopping the server".into(),
                            stderr: false,
                        });
                        break;
                    }
                    "kill" => break,
                    other => {
                        let _ = out_tx.send(OutputLine {
                            line: format!("[test] {other}"),
                            stderr: false,
                        });
                    }
                }
            }
            let _ = exit_tx.send(ExitInfo::clean());
        });
        let kill_tx = stdin_tx.clone();
        ServerHandle::new(
            stdin_tx,
            out_rx,
            exit_rx,
            Box::new(move || {
                let _ = kill_tx.send("kill".into());
            }),
        )
    }

    fn runtime_sources(&self, _java_major: u32) -> Vec<RuntimeSource> {
        vec![RuntimeSource::TermuxRepo { mirror: None }]
    }
}
