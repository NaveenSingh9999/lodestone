pub mod budget;
pub mod config;

use serde::{Deserialize, Serialize};

use crate::supervisor::InstanceState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceSummary {
    pub id: String,
    pub name: String,
    pub server_type: String,
    pub version: String,
    pub state: InstanceState,
}
