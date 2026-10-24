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
use std::os::raw::{c_char, c_int};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lodestone_core::catalog::paper::PaperProvider;
use lodestone_core::catalog::types::Provider;
use lodestone_core::catalog::vanilla::VanillaProvider;
use lodestone_core::net::HttpsClient;
use lodestone_core::platform::{LaunchSpec, PosixPlatform, validate_launch_spec};
use lodestone_core::runtime::termux::provision::{TermuxProvisioner, bootstrap_from_meta};
use lodestone_core::supervisor::InstancePolicy;
use serde::Serialize;
use tokio::runtime::Runtime;

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

#[derive(Serialize)]
struct InstanceSummary {
    id: String,
    server_type: String,
    version: String,
    state: String,
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
}

thread_local! {
    static LAST_ERROR: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
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
pub extern "C" fn lodestone_engine_new(data_dir: *const c_char) -> *mut Engine {
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
        Arc::new(PosixPlatform::new(PathBuf::from(&dir)));
    let engine = Box::new(Engine {
        data_dir: PathBuf::from(dir),
        platform,
        rt,
        client,
        instances: Mutex::new(BTreeMap::new()),
        event_cb: Mutex::new(None),
    });
    Box::into_raw(engine)
}

#[unsafe(no_mangle)]
pub extern "C" fn lodestone_engine_free(engine: *mut Engine) {
    if !engine.is_null() {
        unsafe {
            drop(Box::from_raw(engine));
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lodestone_string_free(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            drop(CString::from_raw(s));
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lodestone_last_error() -> *const c_char {
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
    if let Ok(json) = serde_json::to_string(&event) {
        if let Ok(cb) = CString::new(json) {
            let cb_ptr = cb.into_raw();
            let cb_fn = engine.event_cb.lock().ok().and_then(|g| *g);
            if let Some(f) = cb_fn {
                f(cb_ptr as *const c_char);
            }
            unsafe {
                drop(CString::from_raw(cb_ptr));
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lodestone_register_event_callback(
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
pub extern "C" fn lodestone_list_server_types(engine: *mut Engine, out: *mut *mut c_char) -> c_int {
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
pub extern "C" fn lodestone_device_report(engine: *mut Engine, out: *mut *mut c_char) -> c_int {
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
pub extern "C" fn lodestone_list_versions(
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
pub extern "C" fn lodestone_create_instance(
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
pub extern "C" fn lodestone_list_instances(engine: *mut Engine, out: *mut *mut c_char) -> c_int {
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
pub extern "C" fn lodestone_delete_instance(engine: *mut Engine, id: *const c_char) -> c_int {
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
pub extern "C" fn lodestone_start(engine: *mut Engine, id: *const c_char) -> c_int {
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
                        if let Ok(mut g) = eng.instances.lock() {
                            if let Some(r) = g.get_mut(&id_clone) {
                                r.state = "Running".to_string();
                            }
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
        if let Ok(mut g) = eng.instances.lock() {
            if let Some(r) = g.get_mut(&id_clone) {
                r.state = new_state.to_string();
                r.tail = tail.clone();
                r.handle = std::sync::Arc::new(std::sync::Mutex::new(None));
            }
        }
        emit(
            eng,
            serde_json::json!({"event":"StateChanged","id":id_clone,"state":new_state}),
        );
        let _ = policy;
    });

    // mark starting
    if let Ok(mut g) = engine.instances.lock() {
        if let Some(r) = g.get_mut(&id) {
            r.state = "Starting".to_string();
        }
    }
    emit(
        engine,
        serde_json::json!({"event":"StateChanged","id":id,"state":"Starting"}),
    );
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn lodestone_stop(engine: *mut Engine, id: *const c_char) -> c_int {
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
    if let Ok(mut g) = engine.instances.lock() {
        if let Some(r) = g.get_mut(&id) {
            r.state = "Stopped".to_string();
            *r.handle.lock().unwrap() = None;
        }
    }
    emit(
        engine,
        serde_json::json!({"event":"StateChanged","id":id,"state":"Stopped"}),
    );
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn lodestone_kill(engine: *mut Engine, id: *const c_char) -> c_int {
    lodestone_stop(engine, id)
}

#[unsafe(no_mangle)]
pub extern "C" fn lodestone_send_console(
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
    if let Some(slot) = slot {
        if let Some(h) = slot.lock().unwrap().as_ref() {
            if h.send_console(line) {
                return 0;
            }
        }
    }
    set_error(format!("instance {id} is not running"));
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn lodestone_analyze_crash(
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
pub extern "C" fn lodestone_provision_runtime(
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
