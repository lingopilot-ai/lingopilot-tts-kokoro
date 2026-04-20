use std::path::PathBuf;
use std::time::Instant;

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
    let sym: Result<libloading::Symbol<unsafe extern "C" fn() -> *const core::ffi::c_void>, _> =
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
