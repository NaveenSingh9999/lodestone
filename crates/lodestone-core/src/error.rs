use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("catalog: {0}")]
    Catalog(#[from] CatalogError),
    #[error("network: {0}")]
    Net(#[from] NetError),
    #[error("runtime: {0}")]
    Runtime(#[from] RuntimeError),
    #[error("instance: {0}")]
    Instance(#[from] InstanceError),
    #[error("supervisor: {0}")]
    Supervisor(#[from] SupervisorError),
    #[error("storage: {0}")]
    Storage(#[from] StorageError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("no matching version: {server_type} {version}")]
    VersionNotFound {
        server_type: String,
        version: String,
    },
    #[error("server type {0} is not supported")]
    UnsupportedServerType(String),
    #[error("download host {0} is not in the allowlist")]
    HostNotAllowed(String),
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("server type {0} is not available")]
    NotAvailable(String),
    #[error("catalog source error: {0}")]
    Source(String),
    #[error("network: {0}")]
    Http(#[from] NetError),
}

#[derive(Debug, Error)]
pub enum NetError {
    #[error("http client error: {0}")]
    Http(String),
    #[error("url {0} is not https or is not in the host allowlist")]
    DisallowedHost(String),
    #[error("response exceeds the size limit")]
    TooLarge,
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("no runtime source supplies Java {0}")]
    NoSourceForMajor(u32),
    #[error(
        "runtime failed the page size gate: device requires {required} bytes, worst ELF segment is aligned to {found}"
    )]
    PageSizeGateFailed { required: u32, found: u64 },
    #[error("runtime {0} is not installed")]
    NotInstalled(String),
    #[error("provisioning failed: {0}")]
    Provision(String),
    #[error("runtime smoke test failed: {0}")]
    SmokeTest(String),
}

#[derive(Debug, Error)]
pub enum InstanceError {
    #[error("instance {0} not found")]
    NotFound(String),
    #[error("instance {0} already exists")]
    AlreadyExists(String),
    #[error("invalid config: {0}")]
    Config(String),
    #[error("resource budget exceeded: {0}")]
    BudgetExceeded(String),
    #[error("heap budget {computed_mb} MB is below the {minimum_mb} MB workable minimum")]
    HeapTooSmall { computed_mb: u64, minimum_mb: u64 },
}

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error("instance {0} already running")]
    AlreadyRunning(String),
    #[error("instance {0} is not running")]
    NotRunning(String),
    #[error("process spawn failed: {0}")]
    Spawn(String),
    #[error("eula is not accepted for instance {0}")]
    EulaNotAccepted(String),
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("path escape: {0}")]
    PathEscape(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
