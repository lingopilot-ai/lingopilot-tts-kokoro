use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::path::PathBuf;
use std::time::Instant;

#[repr(C)]
struct OrtApiBase {
    get_api: unsafe extern "C" fn(u32) -> *const c_void,
    get_version_string: unsafe extern "C" fn() -> *const c_char,
}

fn ts() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("{}.{:09}", d.as_secs(), d.subsec_nanos())
}

fn log(stage: &str, extra: &str) {
    println!("[probe] ts={} stage={} {}", ts(), stage, extra);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: ort-init-probe <path-to-onnxruntime.dll>");
        std::process::exit(2);
    }
    let dll = PathBuf::from(&args[1]);
    log("start", &format!("dll={}", dll.display()));

    let t0 = Instant::now();
    log("pre_libloading_new", "");
    let lib = unsafe { libloading::Library::new(&dll) };
    match &lib {
        Ok(_) => log(
            "post_libloading_new_ok",
            &format!("elapsed_ms={}", t0.elapsed().as_millis()),
        ),
        Err(e) => {
            log(
                "post_libloading_new_err",
                &format!("elapsed_ms={} error={}", t0.elapsed().as_millis(), e),
            );
            std::process::exit(10);
        }
    }
    let lib = lib.unwrap();

    let t1 = Instant::now();
    log("pre_get_symbol_OrtGetApiBase", "");
    let sym: Result<libloading::Symbol<unsafe extern "C" fn() -> *const OrtApiBase>, _> =
        unsafe { lib.get(b"OrtGetApiBase\0") };
    match &sym {
        Ok(_) => log(
            "post_get_symbol_ok",
            &format!("elapsed_ms={}", t1.elapsed().as_millis()),
        ),
        Err(e) => {
            log(
                "post_get_symbol_err",
                &format!("elapsed_ms={} error={}", t1.elapsed().as_millis(), e),
            );
            std::process::exit(11);
        }
    }
    let t2 = Instant::now();
    log("pre_call_OrtGetApiBase", "");
    let base_ptr = unsafe { (sym.unwrap())() };
    log(
        "post_call_OrtGetApiBase",
        &format!(
            "elapsed_ms={} ptr_null={}",
            t2.elapsed().as_millis(),
            base_ptr.is_null()
        ),
    );
    if base_ptr.is_null() {
        std::process::exit(13);
    }
    let base: &OrtApiBase = unsafe { &*base_ptr };

    let t_v = Instant::now();
    log("pre_GetVersionString", "");
    let vptr = unsafe { (base.get_version_string)() };
    let version_str = if vptr.is_null() {
        "<null>".to_string()
    } else {
        unsafe { CStr::from_ptr(vptr).to_string_lossy().into_owned() }
    };
    log(
        "post_GetVersionString",
        &format!(
            "elapsed_ms={} version=\"{}\"",
            t_v.elapsed().as_millis(),
            version_str
        ),
    );

    for api_ver in [24u32, 23, 22, 21, 20, 18, 16] {
        let t_a = Instant::now();
        log("pre_GetApi", &format!("api_version={}", api_ver));
        let api_ptr = unsafe { (base.get_api)(api_ver) };
        log(
            "post_GetApi",
            &format!(
                "elapsed_ms={} api_version={} ptr_null={}",
                t_a.elapsed().as_millis(),
                api_ver,
                api_ptr.is_null()
            ),
        );
    }

    drop(lib);

    let t3 = Instant::now();
    log("pre_ort_init_from", "");
    let builder = match ort::init_from(&dll) {
        Ok(b) => {
            log(
                "post_ort_init_from_ok",
                &format!("elapsed_ms={}", t3.elapsed().as_millis()),
            );
            b
        }
        Err(e) => {
            log(
                "post_ort_init_from_err",
                &format!("elapsed_ms={} error={}", t3.elapsed().as_millis(), e),
            );
            std::process::exit(12);
        }
    };

    let t4 = Instant::now();
    log("pre_commit", "");
    builder.commit();
    log(
        "post_commit",
        &format!("elapsed_ms={}", t4.elapsed().as_millis()),
    );

    log("end", "ok=true");
}
