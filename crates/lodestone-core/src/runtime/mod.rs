use std::path::PathBuf;

pub mod elf;

pub use elf::check_elf_alignment;

#[derive(Debug, Clone)]
pub enum RuntimeSource {
    TermuxRepo {
        mirror: Option<String>,
    },
    GithubRelease {
        repo: String,
        asset: String,
        pinned_sha256: Option<String>,
    },
    UserSupplied {
        path: PathBuf,
    },
}

impl RuntimeSource {
    pub fn label(&self) -> &'static str {
        match self {
            RuntimeSource::TermuxRepo { .. } => "termux-repo",
            RuntimeSource::GithubRelease { .. } => "github-release",
            RuntimeSource::UserSupplied { .. } => "user-supplied",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimePlan {
    pub java_major: u32,
    pub source: RuntimeSource,
}

pub fn device_page_size() -> Option<u32> {
    let data = std::fs::read("/proc/self/auxv").ok()?;
    auxv_page_size(&data)
}

pub fn auxv_page_size(data: &[u8]) -> Option<u32> {
    const AT_PAGESZ: u64 = 6;
    if data.len() < 16 {
        return None;
    }
    let mut i = 0;
    while i + 16 <= data.len() {
        let key = u64::from_ne_bytes(data[i..i + 8].try_into().ok()?);
        if key == 0 {
            return None;
        }
        let val = u64::from_ne_bytes(data[i + 8..i + 16].try_into().ok()?);
        if key == AT_PAGESZ && val > 0 && val.is_power_of_two() && val <= u32::MAX as u64 {
            return Some(val as u32);
        }
        i += 16;
    }
    None
}
