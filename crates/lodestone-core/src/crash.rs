//! Crash analysis (spec: analyze_crash -> CrashReport). Parses a server log
//! or crash report and extracts the primary exception, the offending thread,
//! the saved crash-report path (if any), and a human-readable remedy hint.
//! No fabrication: if no exception is found, the report says so.

use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct CrashReport {
    pub primary_exception: Option<String>,
    pub thread: Option<String>,
    pub saved_report_path: Option<String>,
    pub stack_head: Vec<String>,
    pub remedy: &'static str,
}

pub fn analyze_log(text: &str) -> CrashReport {
    let mut primary: Option<String> = None;
    let mut thread: Option<String> = None;
    let mut saved: Option<String> = None;
    let mut stack: Vec<String> = Vec::new();
    let mut in_stack = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('[') {
            // thread extraction from "[..] [ThreadName/LEVEL]:"
            if let Some(idx) = rest.find("] [") {
                let after = &rest[idx + 3..];
                if let Some(slash) = after.find('/') {
                    let tname = after[..slash].trim();
                    if !tname.is_empty() && thread.is_none() {
                        // only capture the thread of the first error block
                    }
                    let _ = tname;
                }
            }
        }

        // saved crash report path
        if trimmed.contains("crash report has been saved to:")
            && let Some(idx) = trimmed.find("saved to:")
        {
            saved = Some(trimmed[idx + "saved to:".len()..].trim().to_string());
        }

        // exception header: "java.lang.X" or "XException" at start (possibly after timestamp)
        if primary.is_none()
            && let Some(exc) = extract_exception(trimmed)
        {
            primary = Some(exc);
            in_stack = true;
            continue;
        }

        if in_stack {
            if trimmed.starts_with("at ")
                || trimmed.starts_with("...")
                || trimmed.starts_with("Caused by:")
            {
                if stack.len() < 8 {
                    stack.push(trimmed.to_string());
                }
                if trimmed.starts_with("Caused by:") {
                    // keep going but don't reset primary
                }
            } else if trimmed.is_empty() {
                in_stack = false;
            } else if !trimmed.starts_with('[') {
                // continuation of stack? keep
            } else {
                in_stack = false;
            }
        }
    }

    // thread: prefer the thread named in the first bracketed error line
    for line in text.lines() {
        let trimmed = line.trim();
        if (trimmed.contains("ERROR")
            || trimmed.contains("exception")
            || trimmed.contains("Exception"))
            && let Some(t) = thread_from_bracket(trimmed)
        {
            thread = Some(t);
            break;
        }
    }

    let remedy = remedy_for(primary.as_deref());

    CrashReport {
        primary_exception: primary,
        thread,
        saved_report_path: saved,
        stack_head: stack,
        remedy,
    }
}

fn thread_from_bracket(line: &str) -> Option<String> {
    // "[03:14:07] [Server thread/ERROR]: ..."
    let start = line.find('[')?;
    let rest = &line[start + 1..];
    let close = rest.find(']')?;
    let _inner = &rest[..close]; // "03:14:07"
    // need the second bracket
    let rest2 = &rest[close + 1..];
    let start2 = rest2.find('[')?;
    let rest3 = &rest2[start2 + 1..];
    let close2 = rest3.find(']')?;
    let bracket = &rest3[..close2]; // "Server thread/ERROR"
    let name = bracket.split('/').next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn extract_exception(line: &str) -> Option<String> {
    // Look for a token ending in Exception/Error/Throwable, possibly qualified.
    for token in line.split_whitespace() {
        let t = token.trim_matches(|c: char| c == ':' || c == ',' || c == ')');
        if t.ends_with("Exception") || t.ends_with("Error") || t.ends_with("Throwable") {
            // strip trailing colon content already handled by split
            return Some(t.to_string());
        }
    }
    // Also handle "Caused by: java.lang.X"
    if let Some(idx) = line.find("Caused by:") {
        let rest = &line[idx + "Caused by:".len()..];
        for token in rest.split_whitespace() {
            let t = token.trim_matches(|c: char| c == ':' || c == ',');
            if t.ends_with("Exception") || t.ends_with("Error") || t.ends_with("Throwable") {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn remedy_for(exc: Option<&str>) -> &'static str {
    match exc {
        Some(e) if e.contains("OutOfMemoryError") => {
            "Java heap exhausted. Lower view/simulation distance, reduce heap usage, or close background apps. Do not silently restart; let the user tune."
        }
        Some(e) if e.contains("NullPointerException") => {
            "A mod or plugin dereferenced null. Check the top stack frame and the mod list; update or remove the offending mod."
        }
        Some(e) if e.contains("BindException") || e.contains("AddressAlreadyInUse") => {
            "Port already in use. Change the server port or stop the other process."
        }
        Some(_) => "Inspect the stack head and the saved crash report for the root cause.",
        None => {
            "No exception found in the log; the process may have been killed by the OS (exit 137)."
        }
    }
}

pub fn analyze_file(path: &Path) -> Result<CrashReport, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    Ok(analyze_log(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/crash")
                .join(name),
        )
        .expect("fixture present")
    }

    #[test]
    fn oom_extracted() {
        let r = analyze_log(&fixture("oom.log"));
        let _ = r.primary_exception.as_deref();
        // accept either form
        let exc = r.primary_exception.clone().unwrap();
        assert!(exc.contains("OutOfMemoryError"), "{exc}");
        assert_eq!(r.thread.as_deref(), Some("Server thread"));
        assert!(r.remedy.contains("heap exhausted"));
        assert!(!r.stack_head.is_empty());
    }

    #[test]
    fn npe_with_saved_report() {
        let r = analyze_log(&fixture("npe.log"));
        let exc = r.primary_exception.clone().unwrap();
        assert!(exc.contains("NullPointerException"), "{exc}");
        assert_eq!(
            r.saved_report_path.as_deref(),
            Some("./crash-reports/crash-2026.txt")
        );
    }

    #[test]
    fn no_exception_yields_none() {
        let r = analyze_log("just some info lines\nnothing wrong here\n");
        assert!(r.primary_exception.is_none());
        assert!(r.remedy.contains("No exception found") || r.remedy.contains("killed by the OS"));
    }

    #[test]
    fn analyze_file_works() {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/crash/oom.log");
        let r = analyze_file(&p).unwrap();
        assert!(r.primary_exception.is_some());
    }

    #[test]
    fn missing_file_errors() {
        assert!(analyze_file(std::path::Path::new("/nope/missing.log")).is_err());
    }
}
