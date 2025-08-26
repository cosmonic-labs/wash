use bytes::Bytes;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Workload {
    pub namespace: String,
    pub name: String,
    pub annotations: HashMap<String, String>,
    pub service: Option<Service>,
    pub wit_world: Option<WitWorld>,
    pub volumes: Vec<Volume>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadState {
    Unspecified = 0,
    Starting = 1,
    Running = 2,
    Completed = 3,
    Stopping = 4,
    Error = 5,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Service {
    pub bytes: Bytes,
    pub local_resources: LocalResources,
    pub max_restarts: u64,
}

// TODO: Rename this to something else, this is just a collection of components.
#[derive(Debug, Clone, PartialEq)]
pub struct WitWorld {
    pub components: Vec<Component>,
    pub host_interfaces: Vec<crate::WitInterface>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    pub bytes: Bytes,
    pub local_resources: LocalResources,
    pub pool_size: i32,
    pub max_invocations: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalResources {
    pub memory_limit_mb: i32,
    pub cpu_limit: i32,
    pub config: HashMap<String, String>,
    pub volume_mounts: Vec<VolumeMount>,
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    pub name: String,
    pub volume_type: VolumeType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VolumeType {
    HostPath(HostPathVolume),
    EmptyDir(EmptyDirVolume),
}

#[derive(Debug, Clone, PartialEq)]
pub struct VolumeMount {
    pub name: String,
    pub mount_path: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmptyDirVolume {}

#[derive(Debug, Clone, PartialEq)]
pub struct HostPathVolume {
    pub local_path: String,
}
