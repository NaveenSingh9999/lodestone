mod support;

use lodestone_core::platform::{Platform, ThermalLevel};
use lodestone_core::supervisor::{FsmEvent, InstanceState, step};
use support::TestPlatform;

fn tmp_root(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lode-flow-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn launch_echo_stop_exit_cycle() {
    let platform = TestPlatform::new(tmp_root("echo"));
    let mut handle = platform.launch_server(lodestone_core::platform::LaunchSpec::default());

    handle.send_console("say hi");
    let echoed = handle.recv_output().await.unwrap();
    assert_eq!(echoed.line, "[test] say hi");
    assert!(!echoed.stderr);

    handle.send_console("stop");
    let stopping = handle.recv_output().await.unwrap();
    assert!(stopping.line.contains("Stopping"));

    let exit = handle.take_exit().unwrap().await.unwrap();
    assert_eq!(exit.code, Some(0));
    assert_eq!(exit.kind(), lodestone_core::platform::ExitKind::Clean);
    assert_eq!(exit.kind().remedy_hint(), None);
    assert_eq!(
        step(InstanceState::Stopping, FsmEvent::ProcessExited).unwrap(),
        InstanceState::Stopped
    );
}

#[tokio::test]
async fn keepalive_lifecycle_tracks_running_state() {
    let platform = TestPlatform::new(tmp_root("keepalive"));
    platform.begin_keepalive("server running");
    assert!(platform.keepalive_active());
    platform.end_keepalive();
    assert!(!platform.keepalive_active());
}

#[tokio::test]
async fn device_signals_flow_through_trait() {
    let mut platform = TestPlatform::new(tmp_root("device"));
    platform.set_thermal(ThermalLevel::Moderate);
    assert!(platform.thermal_status().blocks_optional_work());

    platform.set_power_save(true);
    assert!(platform.power_status().power_save);

    let mem = platform.memory_info();
    let budget = lodestone_core::instance::budget::heap_budget_mb(
        mem.total_bytes / 1024 / 1024,
        mem.available_bytes / 1024 / 1024,
    );
    assert!(budget.is_ok());
    let heap = budget.unwrap();
    assert!(heap <= mem.total_bytes / 1024 / 1024);
}

#[tokio::test]
async fn notifications_and_status_reach_the_surface() {
    let mut platform = TestPlatform::new(tmp_root("notify"));
    platform.notify(&lodestone_core::platform::UserNotification {
        title: "crash loop".into(),
        body: "stopped restarting".into(),
    });
    let notification = platform.notifications.try_recv().unwrap();
    assert!(notification.title.contains("crash loop"));

    platform.show_status(&lodestone_core::platform::StatusSummary {
        headline: "Paper running".into(),
        running_instances: vec!["i-1".into()],
        starting_instances: vec![],
    });
    let state = platform.statuses.try_recv().unwrap();
    assert_eq!(state.headline, "Paper running");
}
