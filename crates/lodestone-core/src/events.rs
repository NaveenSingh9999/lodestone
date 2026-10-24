use serde::{Deserialize, Serialize};

use crate::supervisor::InstanceState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Event {
    StateChanged {
        instance_id: String,
        state: InstanceState,
    },
    Progress {
        op_id: String,
        label: String,
        fraction: Option<f64>,
        bytes_done: Option<u64>,
        bytes_total: Option<u64>,
    },
    ConsoleLine {
        instance_id: String,
        line: String,
        level: LogLevel,
    },
    Metrics(MetricsSample),
    DeviceAlert {
        kind: DeviceAlertKind,
        detail: String,
    },
    OperationFinished {
        op_id: String,
        success: bool,
    },
}

impl Event {
    pub fn for_instance(&self) -> Option<&str> {
        match self {
            Event::StateChanged { instance_id, .. } | Event::ConsoleLine { instance_id, .. } => {
                Some(instance_id)
            }
            Event::Metrics(sample) => Some(&sample.instance_id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSample {
    pub instance_id: String,
    pub timestamp_ms: u64,
    pub cpu_percent: Option<f64>,
    pub rss_bytes: Option<u64>,
    pub tps: Option<f64>,
    pub mspt: Option<f64>,
    pub players_online: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceAlertKind {
    ThermalModerate,
    ThermalSevere,
    PowerSave,
    ProcessKilledBySystem,
    StorageLow,
    NetworkDegraded,
}
