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

    // Set up custom TensorFlow Lite library for tflitec on macOS
    if target_os == "macos" {
        println!("cargo:warning=🔍 macOS detected - using custom TensorFlow Lite libraries");

        let arch_dir = if target_arch == "aarch64" {
            "darwin-aarch64"
        } else {
            "darwin-x86_64"
        };

        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let lib_dir = PathBuf::from(manifest_dir).join("libs").join(arch_dir);

        let tflite_lib_path = lib_dir.join("libtensorflowlite_c.dylib");

        println!("cargo:warning=🔍 Debug: target_arch = {}", target_arch);
        println!("cargo:warning=🔍 Debug: arch_dir = {}", arch_dir);
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

            // Tell tflitec to use our custom library for macOS
            let env_var = if target_arch == "aarch64" {
                "TFLITEC_PREBUILT_PATH_AARCH64_APPLE_DARWIN"
            } else {
                "TFLITEC_PREBUILT_PATH_X86_64_APPLE_DARWIN"
            };

            println!(
                "cargo:warning=🔍 Debug: Setting {} = {}",
                env_var,
                tflite_lib_path.display()
            );
            env::set_var(env_var, &tflite_lib_path);

            // Also set the generic one for good measure
            env::set_var("TFLITEC_PREBUILT_PATH", &tflite_lib_path);

            println!("cargo:rustc-link-search=native={}", lib_dir.display());

            // Link the Metal delegate if available (for GPU acceleration)
            if lib_dir
                .join("libtensorflowlite_metal_delegate.dylib")
                .exists()
            {
                println!("cargo:warning=🔗 Metal delegate available for GPU acceleration");
            }
        } else {
            println!("cargo:warning=⚠️  Custom TensorFlow Lite library not found at {}, tflitec will try to build from source", tflite_lib_path.display());
        }
    }
}
