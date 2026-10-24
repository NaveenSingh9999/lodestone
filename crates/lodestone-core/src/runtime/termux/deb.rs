//! Debian `.deb` extraction: `ar` archive parsing, `data.tar.{xz,gz}` payload
//! selection, decompression to disk (memory-flat), and prefix-stripping
//! unpack into a runtime root.
//!
//! Termux packages are prefix-relocated: every data path lives under
//! `./data/data/com.termux/files/usr/`. That constant prefix is dropped so
//! files unpack relative to the runtime root (bin/, etc/, lib/, share/…).

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const AR_MAGIC: &[u8; 8] = b"!<arch>\n";
const TERMUX_PREFIX: &str = "data/data/com.termux/files/usr";

/// A parsed `ar` member of a `.deb`.
#[derive(Debug, PartialEq)]
struct ArMember {
    name: String,
    /// Byte offset of the member payload.
    start: u64,
    len: u64,
}

fn parse_ar_members(reader: &mut impl Read) -> Result<Vec<ArMember>, String> {
    let mut magic = [0u8; 8];
    match reader.read_exact(&mut magic) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err("not a .deb (file too short for ar header)".to_string());
        }
        Err(e) => return Err(format!("deb read: {e}")),
    }
    if &magic != AR_MAGIC {
        return Err("not a .deb (missing ar magic)".to_string());
    }

    let mut members = Vec::new();
    // file cursor offset so far; members are recorded with absolute payload
    // offsets so `member_bytes` can seek instead of walking the archive.
    let mut offset: u64 = AR_MAGIC.len() as u64;
    loop {
        let mut header = [0u8; 60];
        let header = match reader.read(&mut header) {
            Ok(60) => header,
            Ok(0) => break, // clean EOF
            Ok(n) => {
                if header[..n].iter().all(|c| c.is_ascii_whitespace()) {
                    // trailing padding to even size
                    break;
                }
                return Err(format!("truncated ar header ({n} bytes)"));
            }
            Err(e) => return Err(format!("ar read: {e}")),
        };
        let raw_name: String = String::from_utf8_lossy(&header[0..16]).to_string();
        let name = raw_name.trim_end().trim_end_matches('/').to_string();
        let size_str = String::from_utf8_lossy(&header[48..58]).to_string();
        let size: u64 = size_str
            .trim()
            .parse()
            .map_err(|_| format!("bad ar size field: {size_str:?}"))?;
        let start = offset + 60;
        members.push(ArMember {
            name,
            start,
            len: size,
        });
        read_skip(reader, size + (size & 1)).map_err(|e| format!("skip member data: {e}"))?;
        offset = start + size + (size & 1);
    }
    Ok(members)
}

fn read_skip(reader: &mut impl Read, n: u64) -> std::result::Result<(), String> {
    let mut remaining = n;
    let mut buf = [0u8; 8192];
    while remaining > 0 {
        let want = buf.len().min(remaining as usize);
        let got = reader.read(&mut buf[..want]).map_err(|e| e.to_string())?;
        if got == 0 {
            return Err("unexpected EOF in ar member".to_string());
        }
        remaining -= got as u64;
    }
    Ok(())
}

/// Opens the member by name, reading it entirely into memory.
fn member_bytes(path: &Path, member: &ArMember) -> Result<Vec<u8>, String> {
    let mut f = fs::File::open(path).map_err(|e| format!("open deb: {e}"))?;
    use std::io::Seek;
    f.seek(std::io::SeekFrom::Start(member.start))
        .map_err(|e| format!("seek ar member: {e}"))?;
    let mut buf = vec![0u8; member.len as usize];
    f.read_exact(&mut buf)
        .map_err(|e| format!("read member {}: {e}", member.name))?;
    Ok(buf)
}

/// Which data.tar payload type the deb uses.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DataTar {
    Plain,
    Gzip,
    Xz,
}

impl DataTar {
    fn from_member(name: &str) -> Option<Self> {
        if name == "data.tar" {
            Some(DataTar::Plain)
        } else if name == "data.tar.gz" {
            Some(DataTar::Gzip)
        } else if name == "data.tar.xz" {
            Some(DataTar::Xz)
        } else {
            None
        }
    }

    fn extension_hint(&self) -> &'static str {
        match self {
            DataTar::Plain => "tar",
            DataTar::Gzip => "tgz",
            DataTar::Xz => "txz",
        }
    }
}

/// The tar entry names seen in a Termux deb, for diagnostics.
#[derive(Debug)]
pub struct ExtractionReport {
    pub files: u64,
    pub dirs: u64,
    pub symlinks: u64,
    pub prefix_stripped: bool,
}

/// Extracts the data payload of a `.deb` into `out_root`, dropping the Termux
/// absolute prefix. Returns a report summary.
pub fn extract_deb(deb_path: &Path, out_root: &Path) -> Result<ExtractionReport, String> {
    fs::create_dir_all(out_root).map_err(|e| format!("mkdir {}: {e}", out_root.display()))?;
    let tmp = std::env::temp_dir().join(format!(
        "lode-data.tar.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    let result = (|| -> Result<ExtractionReport, String> {
        let mut reader = fs::File::open(deb_path).map_err(|e| format!("open deb: {e}"))?;
        let members = parse_ar_members(&mut reader)?;
        let (candidate, kind) = members
            .iter()
            .find_map(|m| DataTar::from_member(&m.name).map(|k| (m, k)))
            .ok_or_else(|| "no data.tar.* member in deb".to_string())?;

        let payload = member_bytes(deb_path, candidate)?;
        let tar_path = tmp.with_extension(kind.extension_hint());
        let mut out = fs::File::create(&tar_path).map_err(|e| e.to_string())?;
        match kind {
            DataTar::Plain => {
                out.write_all(&payload).map_err(|e| e.to_string())?;
            }
            DataTar::Gzip => {
                let mut dec = flate2::read::GzDecoder::new(payload.as_slice());
                std::io::copy(&mut dec, &mut out).map_err(|e| format!("gunzip: {e}"))?;
            }
            DataTar::Xz => {
                let mut payload_reader = payload.as_slice();
                lzma_rs::xz_decompress(&mut payload_reader, &mut out)
                    .map_err(|e| format!("xz: {e}"))?;
            }
        }
        let report = unpack_tar(&tar_path, out_root)?;
        let _ = fs::remove_file(&tar_path);
        Ok(report)
    })();
    let _ = fs::remove_file(&tmp);
    result
}

fn unpack_tar(tar_path: &Path, out_root: &Path) -> Result<ExtractionReport, String> {
    let f = fs::File::open(tar_path).map_err(|e| format!("open tar: {e}"))?;
    let mut archive = tar::Archive::new(std::io::BufReader::new(f));
    archive.set_preserve_permissions(true);

    let mut report = ExtractionReport {
        files: 0,
        dirs: 0,
        symlinks: 0,
        prefix_stripped: false,
    };

    for entry in archive.entries().map_err(|e| format!("tar entries: {e}"))? {
        let mut entry = entry.map_err(|e| format!("tar entry: {e}"))?;
        let raw = entry
            .path()
            .map_err(|e| format!("tar path: {e}"))?
            .to_path_buf();
        let rel = strip_termux_prefix_fold(&raw);
        if rel.as_os_str().is_empty() {
            continue; // root directory entry
        }
        report.prefix_stripped = raw
            .to_string_lossy()
            .contains("data/data/com.termux/files/usr");
        if is_unsafe(&rel) {
            return Err(format!("unsafe tar path: {}", raw.display()));
        }
        let target = out_root.join(&rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        let header = entry.header();
        match header.entry_type() {
            tar::EntryType::Directory => report.dirs += 1,
            tar::EntryType::Symlink => report.symlinks += 1,
            tar::EntryType::Regular => report.files += 1,
            _ => {
                // hardlinks/other: count with the tar crate's own unpack
            }
        }
        if !header.entry_type().is_dir() {
            entry
                .unpack(&target)
                .map_err(|e| format!("unpack {}: {e}", target.display()))?;
        } else {
            fs::create_dir_all(&target).map_err(|e| format!("mkdir {}: {e}", target.display()))?;
            let m = header.mode().unwrap_or(0o755);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&target, fs::Permissions::from_mode(m));
            }
        }
    }
    Ok(report)
}

/// Removes `./` noise and, when present, the Termux prefix.
fn strip_termux_prefix_fold(raw: &Path) -> PathBuf {
    let comps: Vec<String> = raw
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            std::path::Component::CurDir => None,
            other => other.as_os_str().to_str().map(String::from),
        })
        .collect();
    let joined = comps
        .iter()
        .filter(|c| !c.is_empty())
        .cloned()
        .collect::<Vec<String>>()
        .join("/");

    if let Some(stripped) = joined.strip_prefix(TERMUX_PREFIX) {
        if let Ok(rel) = Path::new(stripped).strip_prefix("/") {
            rel.to_path_buf()
        } else {
            PathBuf::from(stripped.trim_start_matches('/'))
        }
    } else if joined.starts_with('/') {
        PathBuf::from(joined.trim_start_matches('/'))
    } else {
        PathBuf::from(joined)
    }
}

fn is_unsafe(rel: &Path) -> bool {
    rel.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::RootDir
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_strip_variants() {
        assert_eq!(
            strip_termux_prefix_fold(Path::new("./data/data/com.termux/files/usr/bin/java"))
                .to_string_lossy(),
            "bin/java"
        );
        assert_eq!(
            strip_termux_prefix_fold(Path::new("data/data/com.termux/files/usr/etc/x"))
                .to_string_lossy(),
            "etc/x"
        );
        assert_eq!(
            strip_termux_prefix_fold(Path::new("data/data/com.termux/files/usr/"))
                .to_string_lossy(),
            ""
        );
        assert_eq!(
            strip_termux_prefix_fold(Path::new("somewhere/else")).to_string_lossy(),
            "somewhere/else"
        );
        assert!(is_unsafe(Path::new("../outside")));
    }

    #[test]
    fn sha_member_report() {
        let tmp = std::env::temp_dir().join(format!(
            "lode-ccj-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let deb = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/termux/ccj.deb");
        let out = tmp.join("rt");
        let r = extract_deb(&deb, &out);
        match r {
            Ok(rep) => {
                assert!(rep.prefix_stripped);
                // The only file is lib/jvm/java-17-openjdk/lib/security/jssecacerts
                let jsse = out.join("lib/jvm/java-17-openjdk/lib/security/jssecacerts");
                assert!(jsse.exists(), "jssecacerts missing after extract");
                assert_eq!(rep.files, 1);
            }
            Err(e) => panic!("extract failed: {e}"),
        }
        // snag leftover: report on cleanup
        let _ = fs::remove_dir_all(tmp);
    }

    #[test]
    fn not_a_deb() {
        let f = std::env::temp_dir().join("lode-nodeb");
        fs::write(&f, b"garbage").unwrap();
        let e = extract_deb(&f, std::env::temp_dir().as_path()).unwrap_err();
        assert!(e.contains("not a .deb"), "{e}");
        let _ = fs::remove_file(&f);
    }
}
