use std::collections::HashMap;

use crate::{WitInterface, Workload, workload::WorkloadState};

#[derive(Debug, Clone, PartialEq)]
pub struct HostHeartbeat {
    pub id: String,
    pub hostname: String,
    pub friendly_name: String,
    pub version: String,
    pub labels: HashMap<String, String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub os_arch: String,
    pub os_name: String,
    pub os_kernel: String,
    /// System CPU usage in percent (0.0 - 100.0)
    pub system_cpu_usage: f32,
    /// System total memory in bytes
    pub system_memory_total: u64,
    /// System free memory in bytes
    pub system_memory_free: u64,
    pub component_count: u64,
    pub provider_count: u64,
    pub imports: Vec<WitInterface>,
    pub exports: Vec<WitInterface>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadStatus {
    pub workload_id: String,
    pub workload_state: WorkloadState,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadStartRequest {
    pub workload: Workload,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadStartResponse {
    pub workload_status: WorkloadStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadStatusRequest {
    pub workload_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadStatusResponse {
    pub workload_status: WorkloadStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadStopRequest {
    pub workload_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadStopResponse {
    pub workload_status: WorkloadStatus,
}
