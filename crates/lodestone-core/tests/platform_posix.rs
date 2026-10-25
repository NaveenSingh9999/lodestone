//! Integration tests for the POSIX execution surfaces and the supervisor
//! runner, using REAL fork/exec of shell stubs (`bin/java` scripts) instead
//! of fakes — real stdio bridging, real process groups, real signal
//! classification per the Lodestone spec §4.3/§4.4.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lodestone_core::platform::{LaunchSpec, PosixPlatform, validate_launch_spec};
use lodestone_core::supervisor::{InstancePolicy, InstanceRunner, InstanceState, RunOutcome};

fn temp_root(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("lode-posix-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn shell_path() -> &'static str {
    if Path::new("/system/bin/sh").exists() {
        "/system/bin/sh"
    } else {
        "/bin/sh"
    }
}

#[derive(Clone, Copy, Debug)]
enum StubKind {
    ReadyThenStopsOnCommand,
    ReadyThenSleepsForever,
    SnapshotCrash,
    SelfKill,
}

fn write_stub(java_home: &Path, kind: StubKind) {
    let body = match kind {
        StubKind::ReadyThenStopsOnCommand => format!(
            "#!{}\necho starting\necho 'Done (1.000s)!'\nread line\ncase \"$line\" in\n  stop) echo stopping ;;\n  *) exit 3 ;;\nesac\necho stopped\necho done-stopping\nexit 0\n",
            shell_path()
        ),
        StubKind::ReadyThenSleepsForever => format!(
            "#!{}\necho starting\necho 'Done (0.900s)!'\nsleep 300\n",
            shell_path()
        ),
        StubKind::SnapshotCrash => {
            format!("#!{}\necho boom >&2\nexit 1\n", shell_path())
        }
        StubKind::SelfKill => {
            format!("#!{}\nkill -9 $$\n", shell_path())
        }
    };
    let bin = java_home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let java = bin.join("java");
    std::fs::write(&java, body).unwrap();
    std::fs::set_permissions(&java, PermissionsExt::from_mode(0o755)).unwrap();
}

fn make_spec(work: &Path, java_home: &Path) -> LaunchSpec {
    let jar_path = work.join("server.jar");
    std::fs::write(&jar_path, b"description-stubjar").unwrap();
    LaunchSpec {
        instance_id: "posix-test".to_string(),
        java_home: java_home.to_path_buf(),
        jar: jar_path,
        args: vec![],
        env: Default::default(),
        work_dir: work.to_path_buf(),
        rcon_port: None,
        rcon_password: None,
    }
}

fn fast_policy(max_crashes: usize) -> InstancePolicy {
    InstancePolicy {
        ready_marker: "Done (".to_string(),
        stop_command: "stop".to_string(),
        stop_grace: Duration::from_millis(400),
        max_crashes,
        crash_window: Duration::from_secs(600),
        backoff_initial: Duration::from_millis(20),
        backoff_cap: Duration::from_millis(60),
    }
}

fn runner_for(root: &Path, max_crashes: usize) -> InstanceRunner {
    let platform = Arc::new(PosixPlatform::new(root.join("data")));
    InstanceRunner::new(platform, fast_policy(max_crashes), "posix-test")
}

#[tokio::test]
async fn responsive_stop_walks_the_fsm_chain() {
    let root = temp_root("responsive");
    let java_home = root.join("jdk");
    write_stub(&java_home, StubKind::ReadyThenStopsOnCommand);
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let spec = make_spec(&work, &java_home);
    validate_launch_spec(&spec).unwrap();

    let mut runner = runner_for(&root, 3);
    let report = runner
        .run(
            spec,
            Some(Duration::from_millis(150)),
            Some(Duration::from_secs(20)),
        )
        .await
        .unwrap();

    assert!(
        report.outcome.is_ok(),
        "wanted stop, got {:?} tail={:?}",
        report.outcome,
        report.tail
    );
    assert_eq!(report.restarts, 0);
    let states: Vec<InstanceState> = report.phases.iter().map(|(s, _)| *s).collect();
    assert!(
        states.contains(&InstanceState::Running),
        "states={states:?} tail={:?}",
        report.tail
    );
    assert!(states.contains(&InstanceState::Stopping), "{states:?}");
    assert!(states.contains(&InstanceState::Stopped), "{states:?}");
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn crash_loop_reaches_policy_give_up() {
    let root = temp_root("crashloop");
    let java_home = root.join("jdk");
    write_stub(&java_home, StubKind::SnapshotCrash);
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let spec = make_spec(&work, &java_home);

    let mut runner = runner_for(&root, 2);
    let report = runner
        .run(spec, None, Some(Duration::from_secs(20)))
        .await
        .unwrap();

    let outcome = report.outcome.clone();
    match outcome {
        RunOutcome::GaveUpAfterCrashes { last: _, count } => {
            assert_eq!(count, 2);
            assert_eq!(
                report.restarts, 1,
                "second crash is the policy end: {report:?}"
            );
            let states: Vec<InstanceState> = report.phases.iter().map(|(s, _)| *s).collect();
            assert!(states.contains(&InstanceState::Crashed));
        }
        other => panic!("wanted give-up, got {other:?} tail={:?}", report.tail),
    }
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn system_kill_is_never_restarted() {
    let root = temp_root("selfkill");
    let java_home = root.join("jdk");
    write_stub(&java_home, StubKind::SelfKill);
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let spec = make_spec(&work, &java_home);

    let mut runner = runner_for(&root, 5);
    let report = runner
        .run(spec, None, Some(Duration::from_secs(20)))
        .await
        .unwrap();

    let outcome = report.outcome.clone();
    match outcome {
        RunOutcome::KilledBySystem { .. } => {
            assert_eq!(report.restarts, 0, "syskill must NOT trigger restarts");
        }
        other => panic!(
            "wanted killed-by-system, got {other:?} tail={:?}",
            report.tail
        ),
    }
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn hung_server_is_escalated_to_sigkill() {
    let root = temp_root("hung");
    let java_home = root.join("jdk");
    write_stub(&java_home, StubKind::ReadyThenSleepsForever);
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let spec = make_spec(&work, &java_home);

    let mut runner = runner_for(&root, 3);
    let report = runner
        .run(
            spec,
            Some(Duration::from_millis(150)),
            Some(Duration::from_secs(20)),
        )
        .await
        .unwrap();

    assert!(
        report.outcome.is_ok(),
        "grace escalation is still a clean stop, got {:?}",
        report.outcome
    );
    let states: Vec<InstanceState> = report.phases.iter().map(|(s, _)| *s).collect();
    assert!(states.contains(&InstanceState::Stopping));
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn launch_spec_escape_reports_before_spawn() {
    let root = temp_root("escapespec");
    let java_home = root.join("jdk");
    write_stub(&java_home, StubKind::SnapshotCrash);
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    // jar lives OUTSIDE the instance work dir -> validation must refuse
    let mut spec = make_spec(&work, &java_home);
    spec.jar = root.join("outside.jar");
    std::fs::write(root.join("outside.jar"), b"jarbytes").unwrap();

    let mut runner = runner_for(&root, 3);
    let report = runner
        .run(spec, None, Some(Duration::from_secs(5)))
        .await
        .unwrap();

    let outcome = report.outcome.clone();
    match outcome {
        RunOutcome::FailedToStart { reason } => {
            assert!(
                reason.contains("outside the instance directory"),
                "reason: {reason}"
            );
            assert_eq!(report.phases.len(), 1, "no FSM progress must occur");
        }
        other => panic!("wanted validation failure, got {other:?}"),
    }
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn output_bridge_delivers_stdout_lines() {
    let root = temp_root("bridge");
    let java_home = root.join("jdk");
    write_stub(&java_home, StubKind::ReadyThenStopsOnCommand);
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let spec = make_spec(&work, &java_home);

    let mut runner = runner_for(&root, 3);
    let report = runner
        .run(
            spec,
            Some(Duration::from_millis(120)),
            Some(Duration::from_secs(20)),
        )
        .await
        .unwrap();

    assert!(
        report.tail.iter().any(|l| l.contains("Done (1.000s)")),
        "stdout lines must be bridged: {:?}",
        report.tail
    );
    std::fs::remove_dir_all(&root).ok();
}
