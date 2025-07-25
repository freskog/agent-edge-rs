use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    // Set up custom TensorFlow Lite library for tflitec on Linux aarch64
    if target_os == "linux" && target_arch == "aarch64" {
        println!(
            "cargo:warning=🔍 Linux aarch64 detected - using custom TensorFlow Lite libraries"
        );

        let lib_dir = match env::var("TFLITEC_PREBUILT_PATH_AARCH64_UNKNOWN_LINUX_GNU") {
            Ok(path_str) => PathBuf::from(path_str),
            Err(_) => {
                // Default to our custom libraries directory (not the file itself)
                let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
                PathBuf::from(manifest_dir)
                    .join("libs")
                    .join("linux-aarch64")
            }
        };

        let tflite_lib_path = lib_dir.join("libtensorflowlite_c.so");

        println!("cargo:warning=🔍 Debug: lib_dir = {}", lib_dir.display());
        println!(
            "cargo:warning=🔍 Debug: tflite_lib_path = {}",
            tflite_lib_path.display()
        );
        println!(
            "cargo:warning=🔍 Debug: tflite_lib_path.exists() = {}",
            tflite_lib_path.exists()
        );

        if tflite_lib_path.exists() {
            println!(
                "cargo:warning=📚 Using custom TensorFlow Lite library: {}",
                tflite_lib_path.display()
            );

            // Tell tflitec to use our custom library
            println!("cargo:warning=🔍 Debug: Setting TFLITEC_PREBUILT_PATH_AARCH64_UNKNOWN_LINUX_GNU = {}", tflite_lib_path.display());
            env::set_var(
                "TFLITEC_PREBUILT_PATH_AARCH64_UNKNOWN_LINUX_GNU",
                &tflite_lib_path,
            );

            println!("cargo:rustc-link-search=native={}", lib_dir.display());

            // Link the supporting XNNPACK libraries
            if lib_dir.join("libcpuinfo.so").exists() {
                println!("cargo:rustc-link-lib=cpuinfo");
                println!("cargo:warning=🔗 Linking custom libcpuinfo.so");
            }

            if lib_dir.join("libpthreadpool.so").exists() {
                println!("cargo:rustc-link-lib=pthreadpool");
                println!("cargo:warning=🔗 Linking custom libpthreadpool.so");
            }
        } else {
            println!("cargo:warning=⚠️  Custom TensorFlow Lite library not found at {}, tflitec will build from source", tflite_lib_path.display());
        }
    }
}
