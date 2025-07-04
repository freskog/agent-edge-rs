use std::{
    env, fs,
    os::unix::fs as unix_fs,
    path::{Path, PathBuf},
};

fn main() {
    // Only do special handling for aarch64-linux builds (Pi / aarch64 dev-container)
    let target = env::var("TARGET").unwrap_or_default();
    if !(target.starts_with("aarch64") && target.contains("linux")) {
        return;
    }

    /* ------------------------------------------------------------------------
       1.  Validate that both pre-built TFLite libraries exist in the source tree
    -------------------------------------------------------------------------*/
    let project_root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let lib_src_dir = project_root.join("libs/linux-aarch64");
    let pthreadpool_dir = project_root.join("libs");

    let c_api = lib_src_dir.join("libtensorflowlite_c.so");
    let cpp_api = lib_src_dir.join("libtensorflowlite.so");
    let pthreadpool_lib = pthreadpool_dir.join("libpthreadpool.a");

    for lib in [&c_api, &cpp_api] {
        if !lib.exists() {
            panic!(
                "Missing TensorFlow Lite library: {}\n\
                 (expected in libs/linux-aarch64/)",
                lib.display()
            );
        }
    }

    if !pthreadpool_lib.exists() {
        panic!(
            "Missing pthreadpool static library: {}\n\
             (expected in libs/)",
            pthreadpool_lib.display()
        );
    }

    /* ------------------------------------------------------------------------
       2.  Tell rustc how to link against the libraries
    -------------------------------------------------------------------------*/
    println!("cargo:rustc-link-search=native={}", lib_src_dir.display());
    println!(
        "cargo:rustc-link-search=native={}",
        pthreadpool_dir.display()
    );

    // Link pthreadpool statically (before TensorFlow Lite)
    println!("cargo:rustc-link-lib=static=pthreadpool");

    println!("cargo:rustc-link-lib=dylib=tensorflowlite_c");
    println!("cargo:rustc-link-lib=dylib=tensorflowlite");
    println!("cargo:rustc-link-lib=dylib=stdc++");

    // Embed rpath so the binary looks in ./libs/linux-aarch64 and ./libs at runtime
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/libs/linux-aarch64");
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/libs");
    // Disable --as-needed to ensure tensorflow-lite is retained even if referenced later
    println!("cargo:rustc-link-arg=-Wl,--no-as-needed");
    // Link again after disabling as-needed to ensure symbols are resolved
    println!("cargo:rustc-link-lib=dylib=pthreadpool");
    println!("cargo:rustc-link-lib=dylib=tensorflowlite");
    println!("cargo:rustc-link-lib=dylib=tensorflowlite_c");
    // Duplicate link using link-arg to force placement at very end of command
    println!("cargo:rustc-link-arg=-Wl,--whole-archive");
    println!(
        "cargo:rustc-link-arg=-Wl,{}/libpthreadpool.a",
        pthreadpool_dir.display()
    );
    println!("cargo:rustc-link-arg=-Wl,--no-whole-archive");
    println!("cargo:rustc-link-arg=-lcpuinfo");
    println!("cargo:rustc-link-arg=-lpthreadpool");
    println!("cargo:rustc-link-arg=-ltensorflowlite");

    /* ------------------------------------------------------------------------
       3.  Make `cargo run` work (debug & release) by copying/symlinking .so's
           next to the produced binary inside target/{debug|release}/…
    -------------------------------------------------------------------------*/
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    // ../.. = target/<triple>/<profile>
    let bin_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("Failed to locate target profile directory");

    let dest_dir = bin_dir.join("libs/linux-aarch64");
    fs::create_dir_all(&dest_dir).expect("Creating libs/linux-aarch64 in target directory failed");

    for lib in [&c_api, &cpp_api] {
        let dest = dest_dir.join(lib.file_name().unwrap());

        if needs_copy(&lib, &dest) {
            // Prefer a symlink to save space; fall back to copy on failure
            match unix_fs::symlink(&lib, &dest) {
                Ok(_) => {}
                Err(_) => {
                    fs::copy(&lib, &dest)
                        .unwrap_or_else(|e| panic!("Copying {} failed: {e}", lib.display()));
                }
            }
        }
    }

    // Remove all pthreadpool/cpuinfo .so copy logic

    let deps_dir = bin_dir.join("deps/libs/linux-aarch64");
    fs::create_dir_all(&deps_dir).ok();

    for lib in [&c_api, &cpp_api] {
        let dest = deps_dir.join(lib.file_name().unwrap());
        if needs_copy(&lib, &dest) {
            match unix_fs::symlink(&lib, &dest) {
                Ok(_) => {}
                Err(_) => {
                    let _ = fs::copy(&lib, &dest);
                }
            }
        }
    }
}

/// Returns true if dest is missing or differs in size or mtime from src
fn needs_copy(src: &Path, dest: &Path) -> bool {
    match (src.metadata(), dest.metadata()) {
        (Ok(s), Ok(d)) => s.len() != d.len() || s.modified().ok() != d.modified().ok(),
        _ => true,
    }
}
