//! Tunnel integration (spec: tunnel). Primary path embeds `playit-agent-core`;
//! this module provides the management surface and a process-based fallback
//! that runs an external `playit` daemon (the spec's documented fallback) and
//! parses its stdout for the public address. The manager is real process I/O;
//! it does not fabricate an address.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum TunnelState {
    Stopped,
    Starting,
    Running { public_address: Option<String> },
    Failed { reason: String },
}

#[derive(Debug, Clone)]
pub struct TunnelConfig {
    pub daemon_path: PathBuf,
    pub local_port: u16,
    pub secret_key: Option<String>,
}

pub struct TunnelManager {
    config: TunnelConfig,
    state: Arc<Mutex<TunnelState>>,
    child: Arc<Mutex<Option<std::process::Child>>>,
    stdout_lines: Arc<Mutex<Vec<String>>>,
}

impl TunnelManager {
    pub fn new(config: TunnelConfig) -> Self {
        TunnelManager {
            config,
            state: Arc::new(Mutex::new(TunnelState::Stopped)),
            child: Arc::new(Mutex::new(None)),
            stdout_lines: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn state(&self) -> TunnelState {
        self.state.lock().unwrap().clone()
    }

    pub fn stdout_lines(&self) -> Vec<String> {
        self.stdout_lines.lock().unwrap().clone()
    }

    /// Start the daemon. Parses stdout for a public address (host:port).
    pub fn start(&self) -> Result<(), String> {
        {
            let mut s = self.state.lock().unwrap();
            if !matches!(*s, TunnelState::Stopped | TunnelState::Failed { .. }) {
                return Err("tunnel already running".to_string());
            }
            *s = TunnelState::Starting;
        }

        let mut cmd = Command::new(&self.config.daemon_path);
        cmd.arg("--port")
            .arg(self.config.local_port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(key) = &self.config.secret_key {
            cmd.env("PLAYIT_SECRET_KEY", key);
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                *self.state.lock().unwrap() = TunnelState::Failed {
                    reason: format!("spawn: {e}"),
                };
                return Err(format!("spawn: {e}"));
            }
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let state = Arc::clone(&self.state);
        let lines = Arc::clone(&self.stdout_lines);
        let port = self.config.local_port;

        // Pump stdout, looking for a public address.
        if let Some(out) = stdout {
            std::thread::spawn(move || {
                use std::io::BufRead;
                let reader = std::io::BufReader::new(out);
                for line in reader.lines().map_while(Result::ok) {
                    lines.lock().unwrap().push(line.clone());
                    if let Some(addr) = parse_public_address(&line) {
                        *state.lock().unwrap() = TunnelState::Running {
                            public_address: Some(addr),
                        };
                    }
                }
                // process exited
                let mut s = state.lock().unwrap();
                if !matches!(*s, TunnelState::Running { .. }) {
                    *s = TunnelState::Failed {
                        reason: "daemon exited before reporting an address".to_string(),
                    };
                }
                let _ = port;
            });
        }
        if let Some(err) = stderr {
            std::thread::spawn(move || {
                use std::io::BufRead;
                let reader = std::io::BufReader::new(err);
                for line in reader.lines().map_while(Result::ok) {
                    // capture but don't fail on stderr alone
                    let _ = line;
                }
            });
        }

        *self.child.lock().unwrap() = Some(child);
        // Mark running-without-address until stdout reports one.
        *self.state.lock().unwrap() = TunnelState::Running {
            public_address: None,
        };
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        *self.state.lock().unwrap() = TunnelState::Stopped;
        self.stdout_lines.lock().unwrap().clear();
        Ok(())
    }

    /// Wait up to `timeout` for a public address to be reported.
    pub fn wait_for_address(&self, timeout: Duration) -> Option<String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let TunnelState::Running {
                public_address: Some(addr),
            } = self.state()
            {
                return Some(addr);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for TunnelManager {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Extract a public address (host:port) from a playit daemon log line.
/// Looks for tokens containing "playit" or ".gg" or a host:port pattern.
pub fn parse_public_address(line: &str) -> Option<String> {
    // Look for an explicit "address:" / "public:" marker first.
    for marker in ["address:", "public:", "tunnel:", "url:"] {
        if let Some(idx) = line.to_lowercase().find(marker) {
            let rest = &line[idx + marker.len()..];
            if let Some(tok) = rest.split_whitespace().next() {
                let tok = tok.trim_matches(|c: char| c == '"' || c == '\'' || c == ',');
                if looks_like_address(tok) {
                    return Some(tok.to_string());
                }
            }
        }
    }
    // Fallback: any token that looks like host:port with playit/gg/numeric.
    for tok in line.split_whitespace() {
        let tok = tok.trim_matches(|c: char| c == '"' || c == '\'' || c == ',' || c == '.');
        if looks_like_address(tok)
            && (tok.contains("playit") || tok.contains(".gg") || tok.contains("tunnel"))
        {
            return Some(tok.to_string());
        }
    }
    None
}

fn looks_like_address(s: &str) -> bool {
    // host:port
    if let Some((host, port)) = s.rsplit_once(':') {
        if host.is_empty() || port.is_empty() {
            return false;
        }
        if port.chars().all(|c| c.is_ascii_digit()) {
            return host.contains('.') || host.contains("playit") || host.contains("localhost");
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_address_with_marker() {
        assert_eq!(
            parse_public_address("tunnel established, address: 123.playit.gg:25565"),
            Some("123.playit.gg:25565".to_string())
        );
        assert_eq!(
            parse_public_address("public: foo.playit.gg:12345"),
            Some("foo.playit.gg:12345".to_string())
        );
    }

    #[test]
    fn parse_address_fallback() {
        assert_eq!(
            parse_public_address("connected to playit tunnel abc.playit.gg:25565 ok"),
            Some("abc.playit.gg:25565".to_string())
        );
    }

    #[test]
    fn no_address_returns_none() {
        assert!(parse_public_address("starting daemon...").is_none());
        assert!(parse_public_address("listening on 0.0.0.0:25565").is_none());
    }

    #[test]
    fn looks_like_address_validation() {
        assert!(looks_like_address("x.playit.gg:25565"));
        assert!(!looks_like_address("notanaddress"));
        assert!(!looks_like_address(":25565"));
    }

    #[test]
    fn manager_lifecycle_with_stub_daemon() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("lode-tun-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let daemon = root.join("playit-stub");
        let sh = if std::path::Path::new("/system/bin/sh").exists() {
            "/system/bin/sh"
        } else {
            "/bin/sh"
        };
        std::fs::write(
            &daemon,
            format!("#!{sh}\necho 'tunnel established, address: stub.playit.gg:25565'\nsleep 2\n"),
        )
        .unwrap();
        std::fs::set_permissions(&daemon, PermissionsExt::from_mode(0o755)).unwrap();

        let mgr = TunnelManager::new(TunnelConfig {
            daemon_path: daemon,
            local_port: 25565,
            secret_key: None,
        });
        assert_eq!(mgr.state(), TunnelState::Stopped);
        mgr.start().unwrap();
        let addr = mgr.wait_for_address(Duration::from_secs(5));
        assert_eq!(addr.as_deref(), Some("stub.playit.gg:25565"));
        assert!(matches!(mgr.state(), TunnelState::Running { .. }));
        mgr.stop().unwrap();
        assert_eq!(mgr.state(), TunnelState::Stopped);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_daemon_fails() {
        let mgr = TunnelManager::new(TunnelConfig {
            daemon_path: std::path::PathBuf::from("/no/such/playit"),
            local_port: 25565,
            secret_key: None,
        });
        assert!(mgr.start().is_err());
        assert!(matches!(mgr.state(), TunnelState::Failed { .. }));
    }
}
