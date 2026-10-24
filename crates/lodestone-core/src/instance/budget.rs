#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Low,
    Mid,
    High,
}

impl DeviceClass {
    pub fn for_total_ram(total_mb: u64) -> DeviceClass {
        if total_mb < 4096 {
            DeviceClass::Low
        } else if total_mb < 8192 {
            DeviceClass::Mid
        } else {
            DeviceClass::High
        }
    }
}

pub const HEAP_MINIMUM_MB: u64 = 768;

pub fn jvm_budget_mb(total_mb: u64, available_mb: u64) -> u64 {
    let by_total = total_mb as f64 * 0.55;
    let by_available = available_mb.saturating_sub(1024) as f64;
    (f64::min(by_total, by_available)).max(0.0).floor() as u64
}

pub fn heap_budget_mb(
    total_mb: u64,
    available_mb: u64,
) -> Result<u64, crate::error::InstanceError> {
    let budget = jvm_budget_mb(total_mb, available_mb);
    let heap = (budget as f64 * 0.75).floor() as u64;
    if heap < HEAP_MINIMUM_MB {
        return Err(crate::error::InstanceError::HeapTooSmall {
            computed_mb: heap,
            minimum_mb: HEAP_MINIMUM_MB,
        });
    }
    Ok(heap)
}
