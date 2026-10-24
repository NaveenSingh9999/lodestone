use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use lodestone_ffi::{
    lodestone_device_report, lodestone_engine_free, lodestone_engine_new,
    lodestone_list_server_types, lodestone_string_free,
};

fn cstr(ptr: *mut c_char) -> String {
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

#[test]
fn engine_roundtrip() {
    let dir = std::env::temp_dir().join(format!("lode-ffi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let d = CString::new(dir.to_string_lossy().into_owned()).unwrap();
    let engine = unsafe { lodestone_engine_new(d.as_ptr()) as *mut std::ffi::c_void };
    assert!(!engine.is_null());

    let mut out: *mut c_char = std::ptr::null_mut();
    let rc = unsafe { lodestone_list_server_types(engine as *mut _, &mut out) };
    assert_eq!(rc, 0);
    let json = cstr(out);
    unsafe { lodestone_string_free(out) };
    assert!(json.contains("paper"), "{json}");
    assert!(json.contains("vanilla"), "{json}");

    let mut out2: *mut c_char = std::ptr::null_mut();
    let rc = unsafe { lodestone_device_report(engine as *mut _, &mut out2) };
    assert_eq!(rc, 0);
    let rep = cstr(out2);
    unsafe { lodestone_string_free(out2) };
    assert!(rep.contains("total_ram_mb"), "{rep}");

    unsafe { lodestone_engine_free(engine as *mut _) };
    let _ = std::fs::remove_dir_all(&dir);
}
