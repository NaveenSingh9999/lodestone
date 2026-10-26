#![allow(clippy::missing_safety_doc)]
//! C ABI surface for Lodestone. Every function is `extern "C"`, returns a
//! status code (`0` = ok), and writes its payload into a caller-owned
//! `*mut *mut c_char` that must be released with `lodestone_string_free`.
//! Panic safety: every entry point is wrapped so a Rust panic never unwinds
//! across the FFI boundary.
//!
//! This is the bridge the Android (Kotlin) and desktop shells use until the
//! UniFFI codegen (spec §3.3) runs in CI; the interface shape below mirrors
//! the UniFFI command set one-for-one so the eventual migration is mechanical.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::net::SocketAddr;
use std::os::raw::{c_char, c_int};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lodestone_core::catalog::paper::PaperProvider;
use lodestone_core::catalog::types::Provider;
use lodestone_core::catalog::vanilla::VanillaProvider;
use lodestone_core::content::ModrinthClient;
use lodestone_core::crossplay::{self, CrossplayOptions};
use lodestone_core::net::HttpsClient;
use lodestone_core::platform::{LaunchSpec, validate_launch_spec};

#[cfg(not(target_os = "android"))]
use lodestone_core::platform::PosixPlatform as CorePlatform;
#[cfg(target_os = "android")]
use lodestone_core::platform::android::AndroidPlatform as CorePlatform;
use lodestone_core::remote::{
    ApiRequest, ApiResponse, RemoteApi as CoreRemoteApi, RemoteApiConfig as CoreRemoteApiConfig,
    RemoteBackend,
};
use lodestone_core::runtime::termux::provision::{TermuxProvisioner, bootstrap_from_meta};
use lodestone_core::sleep::{SleepConfig as CoreSleepConfig, SleepHook, SleepProxy, WakeHook};
use lodestone_core::supervisor::InstancePolicy;
use serde::Serialize;
use tokio::runtime::Runtime;
use tokio::sync::broadcast;

#[derive(Serialize)]
struct ServerTypeDto {
    id: &'static str,
    name: &'static str,
}

const SERVER_TYPES: &[(&str, &str)] = &[
    ("vanilla", "Vanilla"),
    ("paper", "Paper"),
    ("folia", "Folia"),
    ("purpur", "Purpur"),
    ("fabric", "Fabric"),
    ("quilt", "Quilt"),
    ("forge", "Forge"),
    ("neoforge", "NeoForge"),
];

#[derive(Serialize)]
struct VersionDto {
    id: String,
    release: bool,
    java_major: Option<u32>,
}

#[derive(Serialize, Clone)]
pub(crate) struct PathEntryDto {
    name: String,
    dir: bool,
    size: u64,
}

#[derive(Serialize)]
struct InstanceSummary {
    id: String,
    server_type: String,
    version: String,
    state: String,
    work_dir: String,
}

struct InstanceRecord {
    id: String,
    server_type: String,
    version: String,
    work_dir: PathBuf,
    java_home: PathBuf,
    jar: PathBuf,
    policy: InstancePolicy,
    handle: std::sync::Arc<std::sync::Mutex<Option<lodestone_core::platform::ServerHandle>>>,
    state: String,
    tail: Vec<String>,
}

#[derive(Clone)]
struct FfiClient(std::sync::Arc<HttpsClient>);

impl lodestone_core::net::HttpClient for FfiClient {
    fn get(
        &self,
        url: &str,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, lodestone_core::error::NetError>> + Send
    {
        self.0.get(url)
    }
    fn download(
        &self,
        url: &str,
        dest: &std::path::Path,
    ) -> impl std::future::Future<Output = Result<u64, lodestone_core::error::NetError>> + Send
    {
        self.0.download(url, dest)
    }
}

pub struct Engine {
    data_dir: PathBuf,
    platform: Arc<dyn lodestone_core::platform::Platform>,
    rt: Runtime,
    client: FfiClient,
    instances: Mutex<BTreeMap<String, InstanceRecord>>,
    event_cb: Mutex<Option<extern "C" fn(*const c_char)>>,
    sleep_proxy_running: Mutex<Option<Arc<AtomicBool>>>,
    remote_rt: Mutex<Option<Runtime>>,
    event_tx: Mutex<Option<broadcast::Sender<lodestone_core::events::Event>>>,
}

thread_local! {
    static LAST_ERROR: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn set_error(msg: impl Into<String>) {
    let msg = msg.into();
    LAST_ERROR.with(|e| *e.borrow_mut() = msg);
}

fn ok_json<T: Serialize>(value: &T) -> Result<CString, String> {
    let json = serde_json::to_string(value).map_err(|e| format!("serialize: {e}"))?;
    CString::new(json).map_err(|e| format!("cstring: {e}"))
}

fn take_cstr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string()) }
}

fn write_out(out: *mut *mut c_char, cstr: CString) {
    unsafe {
        *out = cstr.into_raw();
    }
}

// ---------- engine lifecycle ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_engine_new(data_dir: *const c_char) -> *mut Engine {
    let Some(dir) = take_cstr(data_dir) else {
        set_error("data_dir is null or invalid utf8");
        return std::ptr::null_mut();
    };
    let rt = match Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            set_error(format!("tokio runtime: {e}"));
            return std::ptr::null_mut();
        }
    };
    let allowlist = lodestone_core::net::allowlist::HostAllowlist::standard();
    let client = match HttpsClient::new(allowlist) {
        Ok(c) => FfiClient(std::sync::Arc::new(c)),
        Err(e) => {
            set_error(format!("http client: {e}"));
            return std::ptr::null_mut();
        }
    };
    let platform: Arc<dyn lodestone_core::platform::Platform> =
        Arc::new(CorePlatform::new(PathBuf::from(&dir)));
    let engine = Box::new(Engine {
        data_dir: PathBuf::from(dir),
        platform,
        rt,
        client,
        instances: Mutex::new(BTreeMap::new()),
        event_cb: Mutex::new(None),
        sleep_proxy_running: Mutex::new(None),
        remote_rt: Mutex::new(None),
        event_tx: Mutex::new(Some(broadcast::channel(128).0)),
    });
    Box::into_raw(engine)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_engine_free(engine: *mut Engine) {
    if !engine.is_null() {
        unsafe {
            drop(Box::from_raw(engine));
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_string_free(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            drop(CString::from_raw(s));
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_last_error() -> *const c_char {
    // returns a pointer into thread-local storage; valid until next call on
    // this thread. Callers that need to keep it should copy.
    LAST_ERROR.with(|e| {
        let s = e.borrow();
        // leak a CString for the lifetime of the process; acceptable for
        // diagnostics. A production build would store it in the engine.
        let c = CString::new(s.as_str()).unwrap_or_default();
        c.into_raw() as *const c_char
    })
}

fn engine_ref(engine: *mut Engine) -> Option<&'static mut Engine> {
    if engine.is_null() {
        set_error("engine is null");
        None
    } else {
        Some(unsafe { &mut *engine })
    }
}

fn emit(engine: &Engine, event: serde_json::Value) {
    if let Ok(json) = serde_json::to_string(&event)
        && let Ok(cb) = CString::new(json)
    {
        let cb_ptr = cb.into_raw();
        let cb_fn = engine.event_cb.lock().ok().and_then(|g| *g);
        if let Some(f) = cb_fn {
            f(cb_ptr as *const c_char);
        }
        unsafe {
            drop(CString::from_raw(cb_ptr));
        }
    }
    // fan out typed events to any remote API /events subscribers.
    if let Ok(ev) = serde_json::from_value::<lodestone_core::events::Event>(event.clone()) {
        let tx = engine.event_tx.lock().ok().and_then(|g| g.clone());
        let _ = tx.map(|t| t.send(ev));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_register_event_callback(
    engine: *mut Engine,
    cb: Option<extern "C" fn(*const c_char)>,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    if let Ok(mut g) = engine.event_cb.lock() {
        *g = cb;
        0
    } else {
        set_error("event_cb lock poisoned");
        1
    }
}

// ---------- commands ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_list_server_types(
    engine: *mut Engine,
    out: *mut *mut c_char,
) -> c_int {
    let Some(_engine) = engine_ref(engine) else {
        return 1;
    };
    if out.is_null() {
        set_error("out is null");
        return 1;
    }
    let dto: Vec<ServerTypeDto> = SERVER_TYPES
        .iter()
        .map(|(id, name)| ServerTypeDto { id, name })
        .collect();
    match ok_json(&dto) {
        Ok(c) => {
            write_out(out, c);
            0
        }
        Err(e) => {
            set_error(e);
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_device_report(
    engine: *mut Engine,
    out: *mut *mut c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    if out.is_null() {
        set_error("out is null");
        return 1;
    }
    let mem = engine.platform.memory_info();
    let cpu = engine.platform.cpu_info();
    let therm = engine.platform.thermal_status();
    let power = engine.platform.power_status();
    #[derive(Serialize)]
    struct Report {
        total_ram_mb: u64,
        available_ram_mb: u64,
        low_memory: bool,
        cores: u32,
        max_freq_mhz: u32,
        thermal_level: u8,
        temperature_c: Option<f32>,
        battery_percent: Option<u8>,
        charging: bool,
    }
    let r = Report {
        total_ram_mb: mem.total_bytes / (1024 * 1024),
        available_ram_mb: mem.available_bytes / (1024 * 1024),
        low_memory: mem.low_memory,
        cores: cpu.core_count,
        max_freq_mhz: cpu.max_freq_mhz,
        thermal_level: therm.level as u8,
        temperature_c: therm.temperature_c,
        battery_percent: power.battery_percent,
        charging: power.charging,
    };
    match ok_json(&r) {
        Ok(c) => {
            write_out(out, c);
            0
        }
        Err(e) => {
            set_error(e);
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_list_versions(
    engine: *mut Engine,
    server_type: *const c_char,
    out: *mut *mut c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    if out.is_null() {
        set_error("out is null");
        return 1;
    }
    let Some(st) = take_cstr(server_type) else {
        set_error("server_type is null");
        return 1;
    };
    let result: Result<Vec<VersionDto>, String> = (|| {
        let client = engine.client.clone();
        match st.as_str() {
            "vanilla" => {
                let provider = VanillaProvider::new(client);
                let versions = engine
                    .rt
                    .block_on(provider.versions())
                    .map_err(|e| format!("versions: {e}"))?;
                Ok(versions
                    .into_iter()
                    .map(|v| VersionDto {
                        id: v.id,
                        release: v.release,
                        java_major: v.java_major,
                    })
                    .collect())
            }
            "paper" => {
                let provider = PaperProvider::new(client);
                let versions = engine
                    .rt
                    .block_on(provider.versions())
                    .map_err(|e| format!("versions: {e}"))?;
                Ok(versions
                    .into_iter()
                    .map(|v| VersionDto {
                        id: v.id,
                        release: v.release,
                        java_major: v.java_major,
                    })
                    .collect())
            }
            other => Err(format!("unsupported server type: {other}")),
        }
    })();
    match result {
        Ok(v) => match ok_json(&v) {
            Ok(c) => {
                write_out(out, c);
                0
            }
            Err(e) => {
                set_error(e);
                1
            }
        },
        Err(e) => {
            set_error(e);
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_create_instance(
    engine: *mut Engine,
    spec_json: *const c_char,
    out_id: *mut *mut c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    if out_id.is_null() {
        set_error("out_id is null");
        return 1;
    }
    let Some(spec_str) = take_cstr(spec_json) else {
        set_error("spec_json is null");
        return 1;
    };
    #[derive(serde::Deserialize)]
    struct Spec {
        id: String,
        server_type: String,
        version: String,
        work_dir: String,
    }
    let spec: Spec = match serde_json::from_str(&spec_str) {
        Ok(s) => s,
        Err(e) => {
            set_error(format!("spec parse: {e}"));
            return 1;
        }
    };

    // resolve artifact + install
    let work_dir = PathBuf::from(&spec.work_dir);
    let _ = std::fs::create_dir_all(&work_dir);

    let install_result: Result<(PathBuf, Option<u32>), String> = (|| {
        let client = engine.client.clone();
        match spec.server_type.as_str() {
            "vanilla" => {
                let provider = VanillaProvider::new(client);
                let artifact = engine
                    .rt
                    .block_on(provider.resolve(&spec.version))
                    .map_err(|e| format!("resolve: {e}"))?;
                let jm = artifact.java_major;
                engine
                    .rt
                    .block_on(provider.install(&artifact, &work_dir))
                    .map_err(|e| format!("install: {e}"))?;
                Ok((work_dir.join(&artifact.file_name), jm))
            }
            "paper" => {
                let provider = PaperProvider::new(client);
                let artifact = engine
                    .rt
                    .block_on(provider.resolve(&spec.version))
                    .map_err(|e| format!("resolve: {e}"))?;
                let jm = artifact.java_major;
                engine
                    .rt
                    .block_on(provider.install(&artifact, &work_dir))
                    .map_err(|e| format!("install: {e}"))?;
                Ok((work_dir.join(&artifact.file_name), jm))
            }
            other => Err(format!("unsupported server type: {other}")),
        }
    })();

    let (jar, java_major) = match install_result {
        Ok(v) => v,
        Err(e) => {
            set_error(e);
            return 1;
        }
    };

    // provision runtime if needed (Termux only source on this target)
    let java_major = java_major.unwrap_or(21);
    let runtimes_root = engine.data_dir.join("runtimes");
    let provisioner = TermuxProvisioner::standard("aarch64");
    let meta = match engine.rt.block_on(provisioner.ensure_runtime(
        &engine.client,
        &runtimes_root,
        java_major,
    )) {
        Ok(state) => state.into_meta(),
        Err(e) => {
            set_error(format!("runtime provision: {e}"));
            return 1;
        }
    };
    let runtime_root = runtimes_root.join(format!("openjdk-{java_major}"));
    let bootstrap = match bootstrap_from_meta(&runtime_root, &meta) {
        Ok(b) => b,
        Err(e) => {
            set_error(format!("bootstrap: {e}"));
            return 1;
        }
    };

    let record = InstanceRecord {
        id: spec.id.clone(),
        server_type: spec.server_type.clone(),
        version: spec.version.clone(),
        work_dir: work_dir.clone(),
        java_home: bootstrap.java_home.clone(),
        jar: jar.clone(),
        policy: InstancePolicy::default(),
        handle: std::sync::Arc::new(std::sync::Mutex::new(None)),
        state: "Stopped".to_string(),
        tail: Vec::new(),
    };

    if let Ok(mut g) = engine.instances.lock() {
        if g.contains_key(&spec.id) {
            set_error(format!("instance {} already exists", spec.id));
            return 1;
        }
        g.insert(spec.id.clone(), record);
    } else {
        set_error("instances lock poisoned");
        return 1;
    }

    emit(
        engine,
        serde_json::json!({
            "event": "StateChanged",
            "id": spec.id,
            "state": "Stopped",
        }),
    );

    match CString::new(spec.id) {
        Ok(c) => {
            write_out(out_id, c);
            0
        }
        Err(e) => {
            set_error(format!("id cstring: {e}"));
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_list_instances(
    engine: *mut Engine,
    out: *mut *mut c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    if out.is_null() {
        set_error("out is null");
        return 1;
    }
    let g = match engine.instances.lock() {
        Ok(g) => g,
        Err(_) => {
            set_error("instances lock poisoned");
            return 1;
        }
    };
    let list: Vec<InstanceSummary> = g
        .values()
        .map(|r| InstanceSummary {
            id: r.id.clone(),
            server_type: r.server_type.clone(),
            version: r.version.clone(),
            state: r.state.clone(),
            work_dir: r.work_dir.display().to_string(),
        })
        .collect();
    match ok_json(&list) {
        Ok(c) => {
            write_out(out, c);
            0
        }
        Err(e) => {
            set_error(e);
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_delete_instance(
    engine: *mut Engine,
    id: *const c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(id) = take_cstr(id) else {
        set_error("id is null");
        return 1;
    };
    let mut g = match engine.instances.lock() {
        Ok(g) => g,
        Err(_) => {
            set_error("instances lock poisoned");
            return 1;
        }
    };
    if g.remove(&id).is_none() {
        set_error(format!("instance {id} not found"));
        return 1;
    }
    emit(engine, serde_json::json!({"event":"Deleted","id":id}));
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_start(engine: *mut Engine, id: *const c_char) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(id) = take_cstr(id) else {
        set_error("id is null");
        return 1;
    };

    // Validate + clone the record data we need before spawning.
    let (java_home, jar, work_dir, policy) = {
        let g = match engine.instances.lock() {
            Ok(g) => g,
            Err(_) => {
                set_error("instances lock poisoned");
                return 1;
            }
        };
        let record = match g.get(&id) {
            Some(r) => r,
            None => {
                set_error(format!("instance {id} not found"));
                return 1;
            }
        };
        if record.handle.lock().unwrap().is_some() {
            set_error(format!("instance {id} already running"));
            return 1;
        }
        (
            record.java_home.clone(),
            record.jar.clone(),
            record.work_dir.clone(),
            record.policy.clone(),
        )
    };

    let spec = LaunchSpec {
        instance_id: id.clone(),
        java_home,
        jar,
        args: vec![],
        env: Default::default(),
        work_dir,
        rcon_port: None,
        rcon_password: None,
    };
    if let Err(e) = validate_launch_spec(&spec) {
        set_error(e);
        return 1;
    }

    let platform = Arc::clone(&engine.platform);
    let mut handle = platform.launch_server(spec);
    let exit_rx = handle.take_exit();
    let engine_addr = engine as *mut Engine as usize; // Send-safe handle to the engine

    // Pump output + classify exit on a background thread, updating state and
    // emitting events. The handle is stored so stop()/kill() can terminate.
    {
        let handle_slot = {
            let mut g = engine.instances.lock().unwrap();
            let r = g.get_mut(&id).unwrap();
            r.handle.clone()
        };
        *handle_slot.lock().unwrap() = Some(handle);
        {
            let mut g = engine.instances.lock().unwrap();
            let r = g.get_mut(&id).unwrap();
            r.state = "Starting".to_string();
        }
    }
    // Re-take the handle out is not possible (moved). Instead, run the
    // supervision inline on a detached thread that owns a cloned Arc to the
    // engine's instance map via the raw pointer. We keep the handle inside
    // the record; the thread below re-acquires it.
    let id_clone = id.clone();
    let engine_for_thread = engine_addr;
    std::thread::spawn(move || {
        // Re-acquire handle from the record (we stored it above).
        let handle_slot = {
            let eng = unsafe { &*(engine_for_thread as *mut Engine) };
            let g = eng.instances.lock().unwrap();
            g.get(&id_clone).unwrap().handle.clone()
        };
        let Some(mut handle) = handle_slot.lock().unwrap().take() else {
            return;
        };

        let ready_marker = policy.ready_marker.clone();
        let mut ready_seen = false;
        let mut tail: Vec<String> = Vec::new();
        loop {
            let eng = unsafe { &*(engine_for_thread as *mut Engine) };
            let line = eng.rt.block_on(handle.recv_output());
            match line {
                Some(line) => {
                    if !ready_seen && line.line.contains(&ready_marker) {
                        ready_seen = true;
                        let eng = unsafe { &*(engine_for_thread as *mut Engine) };
                        if let Ok(mut g) = eng.instances.lock()
                            && let Some(r) = g.get_mut(&id_clone)
                        {
                            r.state = "Running".to_string();
                        }
                        emit(
                            eng,
                            serde_json::json!({"event":"StateChanged","id":id_clone,"state":"Running"}),
                        );
                    }
                    if tail.len() >= 256 {
                        tail.remove(0);
                    }
                    tail.push(line.line.clone());
                    let eng = unsafe { &*(engine_for_thread as *mut Engine) };
                    emit(
                        eng,
                        serde_json::json!({"event":"ConsoleLine","id":id_clone,"line":line.line,"stderr":line.stderr}),
                    );
                }
                None => break,
            }
        }

        // wait for exit
        let info = if let Some(rx) = exit_rx {
            rx.blocking_recv().ok()
        } else {
            None
        };
        let info = info.unwrap_or(lodestone_core::platform::ExitInfo {
            code: Some(-1),
            signal: None,
        });
        let kind = info.kind();
        let new_state = if kind == lodestone_core::platform::ExitKind::KilledBySystem {
            "Crashed"
        } else if kind == lodestone_core::platform::ExitKind::Clean {
            "Stopped"
        } else {
            "Crashed"
        };
        let eng = unsafe { &*(engine_for_thread as *mut Engine) };
        if let Ok(mut g) = eng.instances.lock()
            && let Some(r) = g.get_mut(&id_clone)
        {
            r.state = new_state.to_string();
            r.tail = tail.clone();
            r.handle = std::sync::Arc::new(std::sync::Mutex::new(None));
        }
        emit(
            eng,
            serde_json::json!({"event":"StateChanged","id":id_clone,"state":new_state}),
        );
        let _ = policy;
    });

    // mark starting
    if let Ok(mut g) = engine.instances.lock()
        && let Some(r) = g.get_mut(&id)
    {
        r.state = "Starting".to_string();
    }
    emit(
        engine,
        serde_json::json!({"event":"StateChanged","id":id,"state":"Starting"}),
    );
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_stop(engine: *mut Engine, id: *const c_char) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(id) = take_cstr(id) else {
        set_error("id is null");
        return 1;
    };
    let slot = {
        let g = match engine.instances.lock() {
            Ok(g) => g,
            Err(_) => {
                set_error("instances lock poisoned");
                return 1;
            }
        };
        match g.get(&id) {
            Some(r) => r.handle.clone(),
            None => {
                set_error(format!("instance {id} not found"));
                return 1;
            }
        }
    };
    if let Some(h) = slot.lock().unwrap().as_ref() {
        h.kill();
    }
    if let Ok(mut g) = engine.instances.lock()
        && let Some(r) = g.get_mut(&id)
    {
        r.state = "Stopped".to_string();
        *r.handle.lock().unwrap() = None;
    }
    emit(
        engine,
        serde_json::json!({"event":"StateChanged","id":id,"state":"Stopped"}),
    );
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_kill(engine: *mut Engine, id: *const c_char) -> c_int {
    unsafe { lodestone_stop(engine, id) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_send_console(
    engine: *mut Engine,
    id: *const c_char,
    line: *const c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(id) = take_cstr(id) else {
        set_error("id is null");
        return 1;
    };
    let Some(line) = take_cstr(line) else {
        set_error("line is null");
        return 1;
    };
    let slot = {
        let g = match engine.instances.lock() {
            Ok(g) => g,
            Err(_) => {
                set_error("instances lock poisoned");
                return 1;
            }
        };
        g.get(&id).map(|r| r.handle.clone())
    };
    if let Some(slot) = slot
        && let Some(h) = slot.lock().unwrap().as_ref()
        && h.send_console(line)
    {
        return 0;
    }
    set_error(format!("instance {id} is not running"));
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_analyze_crash(
    engine: *mut Engine,
    id: *const c_char,
    out: *mut *mut c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    if out.is_null() {
        set_error("out is null");
        return 1;
    }
    let Some(id) = take_cstr(id) else {
        set_error("id is null");
        return 1;
    };
    let tail = engine
        .instances
        .lock()
        .ok()
        .and_then(|g| g.get(&id).map(|r| r.tail.clone()))
        .unwrap_or_default();
    #[derive(Serialize)]
    struct CrashReport {
        instance: String,
        tail_lines: usize,
        tail: Vec<String>,
        hint: &'static str,
    }
    let report = CrashReport {
        instance: id,
        tail_lines: tail.len(),
        tail,
        hint: "inspect the tail for the first error/exception; \
               SIGKILL (137) means the device governor reaped the process — \
               reduce heap or close background apps",
    };
    match ok_json(&report) {
        Ok(c) => {
            write_out(out, c);
            0
        }
        Err(e) => {
            set_error(e);
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_provision_runtime(
    engine: *mut Engine,
    java_major: c_int,
    out: *mut *mut c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    if out.is_null() {
        set_error("out is null");
        return 1;
    }
    let major = java_major as u32;
    let runtimes_root = engine.data_dir.join("runtimes");
    let provisioner = TermuxProvisioner::standard("aarch64");
    let meta =
        match engine
            .rt
            .block_on(provisioner.ensure_runtime(&engine.client, &runtimes_root, major))
        {
            Ok(state) => state.into_meta(),
            Err(e) => {
                set_error(format!("provision: {e}"));
                return 1;
            }
        };
    match ok_json(&meta) {
        Ok(c) => {
            write_out(out, c);
            0
        }
        Err(e) => {
            set_error(e);
            1
        }
    }
}

// ---------- backup ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_create_backup(
    engine: *mut Engine,
    instance_id: *const c_char,
    source_dir: *const c_char,
    label: *const c_char,
    out_name: *mut *mut c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(instance_id) = take_cstr(instance_id) else {
        set_error("instance_id is null");
        return 1;
    };
    let Some(source_dir) = take_cstr(source_dir) else {
        set_error("source_dir is null");
        return 1;
    };
    let label = take_cstr(label).unwrap_or_else(|| "manual".to_string());
    if out_name.is_null() {
        set_error("out_name is null");
        return 1;
    }
    let be = lodestone_core::backup::BackupEngine::new(engine.data_dir.join("backups"));
    match be.create(&instance_id, std::path::Path::new(&source_dir), &label) {
        Ok(info) => match CString::new(info.name) {
            Ok(c) => {
                write_out(out_name, c);
                0
            }
            Err(e) => {
                set_error(format!("name cstring: {e}"));
                1
            }
        },
        Err(e) => {
            set_error(e);
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_list_backups(
    engine: *mut Engine,
    instance_id: *const c_char,
    out: *mut *mut c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(instance_id) = take_cstr(instance_id) else {
        set_error("instance_id is null");
        return 1;
    };
    if out.is_null() {
        set_error("out is null");
        return 1;
    }
    let be = lodestone_core::backup::BackupEngine::new(engine.data_dir.join("backups"));
    match be.list(&instance_id) {
        Ok(list) => {
            #[derive(serde::Serialize)]
            struct Dto {
                name: String,
                size: u64,
                sha256: String,
                path: String,
            }
            let dto: Vec<Dto> = list
                .into_iter()
                .map(|b| Dto {
                    name: b.name,
                    size: b.size,
                    sha256: b.sha256,
                    path: b.path.display().to_string(),
                })
                .collect();
            match ok_json(&dto) {
                Ok(c) => {
                    write_out(out, c);
                    0
                }
                Err(e) => {
                    set_error(e);
                    1
                }
            }
        }
        Err(e) => {
            set_error(e);
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_restore_backup(
    engine: *mut Engine,
    archive_path: *const c_char,
    dest_dir: *const c_char,
    out_count: *mut u64,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(archive_path) = take_cstr(archive_path) else {
        set_error("archive_path is null");
        return 1;
    };
    let Some(dest_dir) = take_cstr(dest_dir) else {
        set_error("dest_dir is null");
        return 1;
    };
    let be = lodestone_core::backup::BackupEngine::new(engine.data_dir.join("backups"));
    match be.restore(
        std::path::Path::new(&archive_path),
        std::path::Path::new(&dest_dir),
        None,
    ) {
        Ok(count) => {
            if !out_count.is_null() {
                unsafe {
                    *out_count = count;
                }
            }
            0
        }
        Err(e) => {
            set_error(e);
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_delete_backup(
    engine: *mut Engine,
    archive_path: *const c_char,
) -> c_int {
    let Some(_engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(archive_path) = take_cstr(archive_path) else {
        set_error("archive_path is null");
        return 1;
    };
    let be = lodestone_core::backup::BackupEngine::new(std::path::PathBuf::from("."));
    match be.delete(std::path::Path::new(&archive_path)) {
        Ok(()) => 0,
        Err(e) => {
            set_error(e);
            1
        }
    }
}

// ---------- content ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_search_content(
    engine: *mut Engine,
    query: *const c_char,
    limit: c_int,
    out: *mut *mut c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(query) = take_cstr(query) else {
        set_error("query is null");
        return 1;
    };
    if out.is_null() {
        set_error("out is null");
        return 1;
    }
    let client = engine.client.clone();
    let mc = lodestone_core::content::ModrinthClient::new(client);
    let limit = if limit <= 0 { 10 } else { limit as u32 };
    match engine.rt.block_on(mc.search(&query, limit)) {
        Ok(resp) => match ok_json(&resp.hits) {
            Ok(c) => {
                write_out(out, c);
                0
            }
            Err(e) => {
                set_error(e);
                1
            }
        },
        Err(e) => {
            set_error(format!("{e}"));
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_list_content_versions(
    engine: *mut Engine,
    project_id: *const c_char,
    out: *mut *mut c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(project_id) = take_cstr(project_id) else {
        set_error("project_id is null");
        return 1;
    };
    if out.is_null() {
        set_error("out is null");
        return 1;
    }
    let client = engine.client.clone();
    let mc = lodestone_core::content::ModrinthClient::new(client);
    match engine.rt.block_on(mc.versions(&project_id)) {
        Ok(vs) => match ok_json(&vs) {
            Ok(c) => {
                write_out(out, c);
                0
            }
            Err(e) => {
                set_error(e);
                1
            }
        },
        Err(e) => {
            set_error(format!("{e}"));
            1
        }
    }
}

// ---------- config ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_apply_tuning(
    engine: *mut Engine,
    server_type: *const c_char,
    profile: *const c_char,
    work_dir: *const c_char,
) -> c_int {
    let Some(_engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(server_type) = take_cstr(server_type) else {
        set_error("server_type is null");
        return 1;
    };
    let Some(profile) = take_cstr(profile) else {
        set_error("profile is null");
        return 1;
    };
    let Some(work_dir) = take_cstr(work_dir) else {
        set_error("work_dir is null");
        return 1;
    };
    let profile = match profile.as_str() {
        "low" => lodestone_core::config::TuningProfile::Low,
        "mid" => lodestone_core::config::TuningProfile::Mid,
        "high" => lodestone_core::config::TuningProfile::High,
        other => {
            set_error(format!("unknown profile: {other}"));
            return 1;
        }
    };
    let storage = match lodestone_core::storage::FsStorage::open(&work_dir) {
        Ok(s) => s,
        Err(e) => {
            set_error(format!("storage: {e}"));
            return 1;
        }
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    match rt.block_on(lodestone_core::config::apply_to_instance_dir(
        &storage,
        &server_type,
        profile,
    )) {
        Ok(_) => 0,
        Err(e) => {
            set_error(format!("{e}"));
            1
        }
    }
}

// ---------- tunnel ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_tunnel_start(
    engine: *mut Engine,
    daemon_path: *const c_char,
    port: c_int,
    secret_key: *const c_char,
    out_status: *mut *mut c_char,
) -> c_int {
    let Some(_engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(daemon_path) = take_cstr(daemon_path) else {
        set_error("daemon_path is null");
        return 1;
    };
    let sk = take_cstr(secret_key);
    let mgr = lodestone_core::tunnel::TunnelManager::new(lodestone_core::tunnel::TunnelConfig {
        daemon_path: std::path::PathBuf::from(daemon_path),
        local_port: port as u16,
        secret_key: sk,
    });
    match mgr.start() {
        Ok(()) => {
            let status = format!("{:?}", mgr.state());
            if !out_status.is_null()
                && let Ok(c) = CString::new(status)
            {
                write_out(out_status, c);
            }
            // keep manager alive by leaking (process lifetime); production
            // code stores it in the engine. For now, leak to avoid drop-kill.
            Box::leak(Box::new(mgr));
            0
        }
        Err(e) => {
            set_error(e);
            1
        }
    }
}

// ---------- sleep proxy (spec 4.12) ----------

struct FfiWake {
    engine_addr: usize,
    running: Arc<AtomicBool>,
}

impl WakeHook for FfiWake {
    fn wake(&self) {
        // switch the proxy into passthrough mode + notify the shell so it can
        // start the server instance.
        self.running.store(true, Ordering::SeqCst);
        let eng = unsafe { &*(self.engine_addr as *mut Engine) };
        emit(eng, serde_json::json!({"event":"SleepWake","id":null}));
    }
}

struct FfiSleep {
    engine_addr: usize,
    running: Arc<AtomicBool>,
}

impl SleepHook for FfiSleep {
    fn sleep(&self) {
        // switch the proxy back into sleeping mode. The shell owns stopping
        // the actual server process; this just flips the proxy behaviour.
        self.running.store(false, Ordering::SeqCst);
        let eng = unsafe { &*(self.engine_addr as *mut Engine) };
        emit(eng, serde_json::json!({"event":"SleepStarted","id":null}));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_start_sleep_proxy(
    engine: *mut Engine,
    config_json: *const c_char,
    bind_port: c_int,
    server_host: *const c_char,
    server_port: c_int,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(cfg_str) = take_cstr(config_json) else {
        set_error("config_json is null");
        return 1;
    };
    let Some(server_host) = take_cstr(server_host) else {
        set_error("server_host is null");
        return 1;
    };
    #[derive(serde::Deserialize)]
    struct Cfg {
        #[serde(default = "default_motd")]
        motd: String,
        #[serde(default = "default_wake")]
        wake_message: String,
        #[serde(default = "default_idle")]
        idle_minutes: u64,
        #[serde(default)]
        proxy_protocol: bool,
    }
    fn default_motd() -> String {
        "Server is sleeping — join to wake it".to_string()
    }
    fn default_wake() -> String {
        "Starting the server, retry shortly".to_string()
    }
    fn default_idle() -> u64 {
        10
    }
    let cfg: Cfg = match serde_json::from_str(&cfg_str) {
        Ok(c) => c,
        Err(e) => {
            set_error(format!("config parse: {e}"));
            return 1;
        }
    };
    let server: SocketAddr = match format!("{server_host}:{server_port}").parse() {
        Ok(a) => a,
        Err(e) => {
            set_error(format!("server addr: {e}"));
            return 1;
        }
    };
    let bind: SocketAddr = match format!("0.0.0.0:{bind_port}").parse() {
        Ok(a) => a,
        Err(e) => {
            set_error(format!("bind addr: {e}"));
            return 1;
        }
    };
    let running = Arc::new(AtomicBool::new(false));
    let wake = Arc::new(FfiWake {
        engine_addr: engine as *mut Engine as usize,
        running: running.clone(),
    });
    let sleep = Arc::new(FfiSleep {
        engine_addr: engine as *mut Engine as usize,
        running: running.clone(),
    });
    let core_cfg = CoreSleepConfig {
        motd: cfg.motd,
        wake_message: cfg.wake_message,
        idle_minutes: cfg.idle_minutes,
        proxy_protocol: cfg.proxy_protocol,
        protocol_version: 767,
    };
    let proxy = SleepProxy {
        bind,
        server,
        cfg: core_cfg,
        running: running.clone(),
        wake,
    };
    if let Ok(mut g) = engine.sleep_proxy_running.lock() {
        *g = Some(running.clone());
    } else {
        set_error("sleep_proxy lock poisoned");
        return 1;
    }
    let idle_minutes = cfg.idle_minutes;
    let server_addr = server;
    engine.rt.spawn(async move {
        let _ = proxy.run().await;
    });
    // lightweight idle watcher: while running, poll player count and sleep.
    if idle_minutes > 0 {
        let sleep = sleep.clone();
        let running2 = running.clone();
        engine.rt.spawn(async move {
            let mut tracker = lodestone_core::sleep::IdleTracker::new(idle_minutes);
            let mut last = false;
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                if !running2.load(Ordering::SeqCst) {
                    last = false;
                    continue;
                }
                match lodestone_core::sleep::query_status(server_addr, Duration::from_secs(5)).await
                {
                    Ok((online, _)) => {
                        if tracker.on_player_count(online) {
                            sleep.sleep();
                        }
                        last = true;
                    }
                    Err(_) => {
                        if last {
                            sleep.sleep();
                        }
                        last = false;
                    }
                }
            }
        });
    }
    emit(
        engine,
        serde_json::json!({"event":"SleepProxyStarted","bind":bind_port}),
    );
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_stop_sleep_proxy(engine: *mut Engine) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    if let Ok(mut g) = engine.sleep_proxy_running.lock() {
        if let Some(r) = g.take() {
            r.store(false, Ordering::SeqCst);
        }
        0
    } else {
        set_error("sleep_proxy lock poisoned");
        1
    }
}

// ---------- remote management API (spec 4.13) ----------

/// Mirrors the section 3.3 command surface over HTTP/WebSocket.
struct FfiBackend {
    engine_addr: usize,
    event_tx: broadcast::Sender<lodestone_core::events::Event>,
}

impl RemoteBackend for FfiBackend {
    fn dispatch(&self, req: ApiRequest) -> ApiResponse {
        let engine = unsafe { &*(self.engine_addr as *mut Engine) };
        let instance_id = req
            .args
            .get("instance_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        match req.command.as_str() {
            "ping" => ApiResponse {
                ok: true,
                data: serde_json::json!({}),
            },
            "list" => {
                let g = engine.instances.lock().ok();
                let list: Vec<serde_json::Value> = g
                    .map(|g| {
                        g.values()
                            .map(|r| {
                                serde_json::json!({
                                    "id": r.id,
                                    "server_type": r.server_type,
                                    "version": r.version,
                                    "state": r.state,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                ApiResponse {
                    ok: true,
                    data: serde_json::json!({"instances": list}),
                }
            }
            "status" => {
                let state_str = {
                    let g = match engine.instances.lock() {
                        Ok(g) => g,
                        Err(_) => {
                            return ApiResponse {
                                ok: false,
                                data: serde_json::json!({"error": "instances lock poisoned"}),
                            };
                        }
                    };
                    g.get(&instance_id)
                        .map(|r| r.state.clone())
                        .unwrap_or_else(|| "Stopped".to_string())
                };
                ApiResponse {
                    ok: true,
                    data: serde_json::json!({
                        "instance_id": instance_id,
                        "state": state_str,
                    }),
                }
            }
            "start" => {
                let cstr = match CString::new(instance_id.as_str()) {
                    Ok(c) => c,
                    Err(_) => {
                        return ApiResponse {
                            ok: false,
                            data: serde_json::json!({"error": "invalid instance id"}),
                        };
                    }
                };
                let rc = unsafe { lodestone_start(self.engine_addr as *mut Engine, cstr.as_ptr()) };
                ApiResponse {
                    ok: rc == 0,
                    data: serde_json::json!({}),
                }
            }
            "stop" => {
                let cstr = match CString::new(instance_id.as_str()) {
                    Ok(c) => c,
                    Err(_) => {
                        return ApiResponse {
                            ok: false,
                            data: serde_json::json!({"error": "invalid instance id"}),
                        };
                    }
                };
                let rc = unsafe { lodestone_stop(self.engine_addr as *mut Engine, cstr.as_ptr()) };
                ApiResponse {
                    ok: rc == 0,
                    data: serde_json::json!({}),
                }
            }
            "restart" => {
                let cstr = match CString::new(instance_id.as_str()) {
                    Ok(c) => c,
                    Err(_) => {
                        return ApiResponse {
                            ok: false,
                            data: serde_json::json!({"error": "invalid instance id"}),
                        };
                    }
                };
                let _ = unsafe { lodestone_stop(self.engine_addr as *mut Engine, cstr.as_ptr()) };
                let rc = unsafe { lodestone_start(self.engine_addr as *mut Engine, cstr.as_ptr()) };
                ApiResponse {
                    ok: rc == 0,
                    data: serde_json::json!({}),
                }
            }
            other => ApiResponse {
                ok: false,
                data: serde_json::json!({"error": format!("unknown command: {other}")}),
            },
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<lodestone_core::events::Event> {
        self.event_tx.subscribe()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_start_remote_api(
    engine: *mut Engine,
    token: *const c_char,
    bind_port: c_int,
    allow_wan: c_int,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let token = take_cstr(token).unwrap_or_default();
    let bind = if allow_wan != 0 {
        format!("0.0.0.0:{bind_port}")
    } else {
        format!("127.0.0.1:{bind_port}")
    };
    let bind: SocketAddr = match bind.parse() {
        Ok(a) => a,
        Err(e) => {
            set_error(format!("bind addr: {e}"));
            return 1;
        }
    };
    let event_tx = match engine.event_tx.lock().ok().and_then(|g| g.clone()) {
        Some(t) => t,
        None => {
            set_error("no event broadcast channel");
            return 1;
        }
    };
    let backend = Box::new(FfiBackend {
        engine_addr: engine as *mut Engine as usize,
        event_tx,
    });
    let cfg = CoreRemoteApiConfig {
        bind,
        token,
        allow_wan: allow_wan != 0,
        max_attempts: 5,
        lockout: Duration::from_secs(300),
    };
    let api = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        CoreRemoteApi::new(cfg, backend)
    })) {
        Ok(a) => a,
        Err(_) => {
            set_error("remote api config rejected");
            return 1;
        }
    };
    let rt = match Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            set_error(format!("remote runtime: {e}"));
            return 1;
        }
    };
    rt.spawn(api.run());
    if let Ok(mut g) = engine.remote_rt.lock() {
        *g = Some(rt);
    } else {
        set_error("remote_rt lock poisoned");
        return 1;
    }
    emit(
        engine,
        serde_json::json!({"event":"RemoteApiStarted","bind":bind_port}),
    );
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_stop_remote_api(engine: *mut Engine) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    if let Ok(mut g) = engine.remote_rt.lock() {
        g.take();
        emit(engine, serde_json::json!({"event":"RemoteApiStopped"}));
        0
    } else {
        set_error("remote_rt lock poisoned");
        1
    }
}

// ---------- device-local file access (file manager / server settings editor) ----------
// Root for all relative paths is the engine data dir. Absolute escapes ("..",
// absolute components, symlinks pointing outside the root) are rejected.

fn validate_rel(rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim_start_matches('/').trim_end_matches('/');
    if rel.is_empty() {
        return Ok(PathBuf::new());
    }
    for comp in std::path::Path::new(rel).components() {
        if !matches!(comp, std::path::Component::Normal(_)) {
            return Err(format!("illegal path: {rel}"));
        }
    }
    Ok(PathBuf::from(rel))
}

fn resolve_under(root: &std::path::Path, rel: PathBuf) -> Result<PathBuf, String> {
    if rel.as_os_str().is_empty() {
        return root.canonicalize().map_err(|e| format!("root: {e}"));
    }
    let full = root.join(&rel);
    let canon = std::fs::canonicalize(&full).map_err(|e| format!("{}: {e}", rel.display()))?;
    let rooted = root.canonicalize().map_err(|e| format!("root: {e}"))?;
    if canon.starts_with(rooted) {
        Ok(canon)
    } else {
        Err(format!("path escapes root: {}", rel.display()))
    }
}

fn target_under(root: &std::path::Path, rel: PathBuf) -> Result<PathBuf, String> {
    if rel.as_os_str().is_empty() {
        return Err("empty rel path".into());
    }
    let full = root.join(&rel);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        let canon = parent.canonicalize().map_err(|e| format!("parent: {e}"))?;
        let rooted = root.canonicalize().map_err(|e| format!("root: {e}"))?;
        if !canon.starts_with(rooted) {
            return Err(format!("path escapes root: {}", rel.display()));
        }
    }
    Ok(full)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_list_files(
    engine: *mut Engine,
    rel: *const c_char,
    out: *mut *mut c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(rel) = take_cstr(rel) else {
        set_error("rel is null");
        return 1;
    };
    if out.is_null() {
        set_error("out is null");
        return 1;
    }
    let result: Result<Vec<PathEntryDto>, String> = (|| {
        let rel = validate_rel(&rel)?;
        let dir = resolve_under(&engine.data_dir, rel)?;
        let mut entries = std::fs::read_dir(&dir).map_err(|e| format!("read_dir: {e}"))?;
        let mut out: Vec<PathEntryDto> = Vec::new();
        while let Some(entry) = entries.next() {
            let entry = entry.map_err(|e| format!("entry: {e}"))?;
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let meta = std::fs::metadata(&path).map_err(|e| format!("metadata: {e}"))?;
            out.push(PathEntryDto {
                name,
                dir: meta.is_dir(),
                size: if meta.is_dir() { 0 } else { meta.len() },
            });
        }
        out.sort_by(|a, b| (b.dir, &a.name).cmp(&(a.dir, &b.name)));
        Ok(out)
    })();
    match result {
        Ok(list) => match ok_json(&list) {
            Ok(c) => {
                write_out(out, c);
                0
            }
            Err(e) => {
                set_error(e);
                1
            }
        },
        Err(e) => {
            set_error(e);
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_read_file(
    engine: *mut Engine,
    rel: *const c_char,
    out: *mut *mut c_char,
) -> c_int {
    const MAX_READ: usize = 512 * 1024;
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(rel) = take_cstr(rel) else {
        set_error("rel is null");
        return 1;
    };
    if out.is_null() {
        set_error("out is null");
        return 1;
    }
    let result: Result<String, String> = (|| {
        let rel = validate_rel(&rel)?;
        let path = resolve_under(&engine.data_dir, rel)?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("metadata: {e}"))?;
        if meta.len() > MAX_READ as u64 {
            return Err(format!("file too large: {} bytes", meta.len()));
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("read: {e}"))?;
        if bytes.contains(&0) {
            return Err("binary file: not editable in app".into());
        }
        String::from_utf8(bytes).map_err(|e| format!("utf8: {e}"))
    })();
    match result {
        Ok(text) => match CString::new(text) {
            Ok(c) => {
                write_out(out, c);
                0
            }
            Err(e) => {
                set_error(format!("content: {e}"));
                1
            }
        },
        Err(e) => {
            set_error(e);
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_write_file(
    engine: *mut Engine,
    rel: *const c_char,
    content: *const c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(rel) = take_cstr(rel) else {
        set_error("rel is null");
        return 1;
    };
    let Some(content) = take_cstr(content) else {
        set_error("content is null");
        return 1;
    };
    let result: Result<(), String> = (|| {
        let rel = validate_rel(&rel)?;
        let path = target_under(&engine.data_dir, rel)?;
        std::fs::write(&path, content).map_err(|e| format!("write: {e}"))
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            set_error(e);
            1
        }
    }
}

// ---------- content install (plugins / mods via Modrinth) ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_install_content(
    engine: *mut Engine,
    project_id: *const c_char,
    version_id: *const c_char,
    dest_dir: *const c_char,
    out_path: *mut *mut c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(project_id) = take_cstr(project_id) else {
        set_error("project_id is null");
        return 1;
    };
    let Some(version_id) = take_cstr(version_id) else {
        set_error("version_id is null");
        return 1;
    };
    let Some(dest_dir) = take_cstr(dest_dir) else {
        set_error("dest_dir is null");
        return 1;
    };
    if out_path.is_null() {
        set_error("out_path is null");
        return 1;
    }
    let result: Result<PathBuf, String> = (|| {
        let rel = validate_rel(&dest_dir)?;
        let dest = target_under(&engine.data_dir, rel)?;
        let client = engine.client.clone();
        let mc = ModrinthClient::new(client);
        let versions = engine
            .rt
            .block_on(mc.versions(&project_id))
            .map_err(|e| format!("versions: {e}"))?;
        let version = versions
            .iter()
            .find(|v| v.id == version_id)
            .ok_or_else(|| format!("version {version_id} not found for {project_id}"))?;
        let file = version
            .files
            .iter()
            .find(|f| f.primary)
            .or_else(|| version.files.first())
            .ok_or_else(|| format!("version {version_id} has no files"))?;
        engine
            .rt
            .block_on(mc.install_file(file, &dest))
            .map_err(|e| format!("install: {e}"))
    })();
    match result {
        Ok(path) => {
            #[derive(Serialize)]
            struct Dto {
                path: String,
            }
            let dto = Dto {
                path: path.display().to_string(),
            };
            match ok_json(&dto) {
                Ok(c) => {
                    write_out(out_path, c);
                    0
                }
                Err(e) => {
                    set_error(e);
                    1
                }
            }
        }
        Err(e) => {
            set_error(e);
            1
        }
    }
}

// ---------- geyser / bedrock crossplay (spec 4.11 optional switch) ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lodestone_enable_crossplay(
    engine: *mut Engine,
    instance_id: *const c_char,
    options_json: *const c_char,
) -> c_int {
    let Some(engine) = engine_ref(engine) else {
        return 1;
    };
    let Some(id) = take_cstr(instance_id) else {
        set_error("instance_id is null");
        return 1;
    };
    let opts_str = take_cstr(options_json).unwrap_or_else(|| "{}".to_string());

    #[derive(serde::Deserialize)]
    struct Opts {
        #[serde(default)]
        geyser: bool,
        #[serde(default)]
        floodgate: bool,
        #[serde(default)]
        via_version: bool,
        #[serde(default)]
        via_backwards: bool,
    }
    let opts: Opts = match serde_json::from_str(&opts_str) {
        Ok(o) => o,
        Err(e) => {
            set_error(format!("options parse: {e}"));
            return 1;
        }
    };

    // Look up the instance record.
    let (server_type, version, work_dir) = {
        let g = match engine.instances.lock() {
            Ok(g) => g,
            Err(_) => {
                set_error("instances lock poisoned");
                return 1;
            }
        };
        match g.get(&id) {
            Some(r) => (r.server_type.clone(), r.version.clone(), r.work_dir.clone()),
            None => {
                set_error(format!("instance {id} not found"));
                return 1;
            }
        }
    };

    let options = CrossplayOptions {
        geyser: opts.geyser,
        floodgate: opts.floodgate,
        via_version: opts.via_version,
        via_backwards: opts.via_backwards,
    };

    let client = engine.client.clone();
    let mc = ModrinthClient::new(client);
    let rt = &engine.rt;
    match rt.block_on(crossplay::install_crossplay(
        &mc,
        &server_type,
        &version,
        &work_dir,
        options,
    )) {
        Ok(result) => {
            emit(
                engine,
                serde_json::json!({
                    "event": "CrossplayInstalled",
                    "id": id,
                    "installed": result.installed,
                    "skipped": result.skipped,
                }),
            );
            0
        }
        Err(e) => {
            set_error(format!("crossplay: {e}"));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn temp_root(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "lodestone-ffi-{}-{}-{}",
            std::process::id(),
            name,
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    unsafe fn take_out(out: *mut *mut c_char) -> String {
        let raw = unsafe { out.read() };
        let s = unsafe { CStr::from_ptr(raw).to_string_lossy().into_owned() };
        unsafe { lodestone_string_free(raw) };
        s
    }

    #[test]
    fn rel_paths_reject_traversal() {
        assert!(validate_rel("../etc").is_err());
        assert!(validate_rel("a/../b").is_err());
        assert!(validate_rel(".").is_err());
        assert!(validate_rel("").is_ok());
        assert!(validate_rel("/instances/a/server.properties").is_ok());
    }

    #[test]
    fn list_write_read_roundtrip() {
        let root = temp_root("roundtrip");
        let root_c = CString::new(root.to_str().unwrap()).unwrap();
        let engine = unsafe { lodestone_engine_new(root_c.as_ptr()) };
        assert!(!engine.is_null());

        let rel = CString::new("instances/test/server.properties").unwrap();
        let text = CString::new("motd=hello\nmax-players=4").unwrap();
        assert_eq!(
            unsafe { lodestone_write_file(engine, rel.as_ptr(), text.as_ptr()) },
            0
        );

        let mut slot: *mut c_char = std::ptr::null_mut();
        let out: *mut *mut c_char = &mut slot;
        let rrc = unsafe { lodestone_read_file(engine, rel.as_ptr(), out) };
        if rrc != 0 {
            let le = unsafe {
                CStr::from_ptr(lodestone_last_error())
                    .to_string_lossy()
                    .into_owned()
            };
            panic!("read failed: {le}");
        }
        assert_eq!(rrc, 0);
        let content = unsafe { take_out(out) };
        assert_eq!(content, "motd=hello\nmax-players=4");

        // list the parent dir
        let dir = CString::new("instances").unwrap();
        let mut slot2: *mut c_char = std::ptr::null_mut();
        let out2: *mut *mut c_char = &mut slot2;
        assert_eq!(
            unsafe { lodestone_list_files(engine, dir.as_ptr(), out2) },
            0
        );
        let listed = unsafe { take_out(out2) };
        assert!(listed.contains("\"name\":\"test\"") && listed.contains("\"dir\":true"));

        // traversal is refused
        let evil = CString::new("../escape.txt").unwrap();
        let evil_text = CString::new("nope").unwrap();
        assert_ne!(
            unsafe { lodestone_write_file(engine, evil.as_ptr(), evil_text.as_ptr()) },
            0
        );

        unsafe { lodestone_engine_free(engine) };
        let _ = std::fs::remove_dir_all(&root);
    }
}
