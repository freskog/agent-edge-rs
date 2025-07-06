use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-check-cfg=cfg(xnnpack)");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    // Only do custom stuff on Linux aarch64 - everywhere else let tflitec handle everything
    if target_os == "linux" && target_arch == "aarch64" {
        println!("cargo:warning=🔍 Linux aarch64 detected - applying custom XNNPACK configuration");

        // Always link pthread on Unix systems
        if target_os != "windows" {
            println!("cargo:rustc-link-lib=pthread");
        }

        // Link our custom XNNPACK libraries if available
        let has_custom_libs = link_custom_xnnpack_libraries();

        // Only generate stub implementations if we don't have real libraries
        if !has_custom_libs {
            println!("cargo:warning=🔄 Generating weak stubs as fallback");
            generate_stubs();
        }
    } else {
        println!(
            "cargo:warning=🔍 Platform {} {} - using tflitec defaults (no custom configuration)",
            target_os, target_arch
        );
        // Do absolutely nothing - let tflitec handle everything normally
    }
}

fn link_custom_xnnpack_libraries() -> bool {
    let lib_dir = "libs/linux-aarch64";
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let full_lib_path = PathBuf::from(&manifest_dir).join(lib_dir);

    if full_lib_path.exists() {
        println!(
            "cargo:warning=📂 Custom XNNPACK library directory found: {}",
            full_lib_path.display()
        );

        // Add library search path for our custom libraries
        println!("cargo:rustc-link-search=native={}", full_lib_path.display());

        // Add rpath for runtime linking
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,{}",
            full_lib_path.display()
        );

        // Link our custom XNNPACK libraries if they exist
        let mut linked_libs = Vec::new();

        if full_lib_path.join("libcpuinfo.so").exists() {
            println!("cargo:rustc-link-lib=cpuinfo");
            println!("cargo:warning=🔗 Linking custom libcpuinfo.so");
            linked_libs.push("libcpuinfo.so");
        }

        if full_lib_path.join("libpthreadpool.so").exists() {
            println!("cargo:rustc-link-lib=pthreadpool");
            println!("cargo:warning=🔗 Linking custom libpthreadpool.so");
            linked_libs.push("libpthreadpool.so");
        }

        // Link custom TensorFlow Lite if available (overrides tflitec's version)
        if full_lib_path.join("libtensorflowlite_c.so").exists() {
            println!("cargo:rustc-link-lib=tensorflowlite_c");
            println!("cargo:warning=🔗 Linking custom libtensorflowlite_c.so");
            linked_libs.push("libtensorflowlite_c.so");
        }

        if !linked_libs.is_empty() {
            println!("cargo:rustc-cfg=xnnpack");
            println!(
                "cargo:warning=🚀 Custom XNNPACK libraries linked: {}",
                linked_libs.join(", ")
            );
            true
        } else {
            println!("cargo:warning=⚠️  No custom XNNPACK libraries found - using tflitec default");
            false
        }
    } else {
        println!("cargo:warning=⚠️  Custom XNNPACK library directory not found: {} - using tflitec default", full_lib_path.display());
        false
    }
}

fn generate_stubs() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = PathBuf::from(&out_dir).join("weak_stubs.c");

    let stub_code = r#"
// Weak stub implementations for cpuinfo and pthreadpool functions
// These will be used if the real libraries are not available

#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>

// cpuinfo weak stubs
__attribute__((weak)) int cpuinfo_initialize(void) {
    return 1; // Success
}

__attribute__((weak)) void cpuinfo_deinitialize(void) {
    // No-op
}

__attribute__((weak)) uint32_t cpuinfo_get_cores_count(void) {
    return 1; // Single core fallback
}

__attribute__((weak)) uint32_t cpuinfo_get_processors_count(void) {
    return 1; // Single processor fallback
}

__attribute__((weak)) int cpuinfo_has_x86_sse(void) {
    return 0; // Conservative fallback - no SIMD
}

__attribute__((weak)) int cpuinfo_has_x86_sse2(void) {
    return 0; // Conservative fallback - no SIMD
}

__attribute__((weak)) int cpuinfo_has_x86_avx(void) {
    return 0; // Conservative fallback - no SIMD
}

__attribute__((weak)) int cpuinfo_has_x86_avx2(void) {
    return 0; // Conservative fallback - no SIMD
}

__attribute__((weak)) int cpuinfo_has_arm_neon(void) {
    return 0; // Conservative fallback - no SIMD
}

// pthreadpool weak stubs
typedef struct pthreadpool* pthreadpool_t;
typedef void (*pthreadpool_task_1d_t)(void*, size_t);
typedef void (*pthreadpool_task_1d_tile_1d_t)(void*, size_t, size_t);
typedef void (*pthreadpool_task_2d_t)(void*, size_t, size_t);
typedef void (*pthreadpool_task_2d_tile_1d_t)(void*, size_t, size_t, size_t);
typedef void (*pthreadpool_task_2d_tile_2d_t)(void*, size_t, size_t, size_t, size_t);

__attribute__((weak)) pthreadpool_t pthreadpool_create(size_t threads_count) {
    // Return a fake pointer to indicate "success" but single-threaded mode
    return (pthreadpool_t)1;
}

__attribute__((weak)) void pthreadpool_destroy(pthreadpool_t threadpool) {
    // No-op
}

__attribute__((weak)) void pthreadpool_parallelize_1d(
    pthreadpool_t threadpool,
    pthreadpool_task_1d_t function,
    void* argument,
    size_t range,
    size_t tile) {
    // Fallback to single-threaded execution
    if (function && range > 0) {
        for (size_t i = 0; i < range; i++) {
            function(argument, i);
        }
    }
}

__attribute__((weak)) void pthreadpool_parallelize_1d_tile_1d(
    pthreadpool_t threadpool,
    pthreadpool_task_1d_tile_1d_t function,
    void* argument,
    size_t range,
    size_t tile) {
    // Fallback to single-threaded execution
    if (function && range > 0) {
        size_t tile_size = (tile > 0) ? tile : 1;
        for (size_t i = 0; i < range; i += tile_size) {
            size_t actual_tile = (i + tile_size <= range) ? tile_size : (range - i);
            function(argument, i, actual_tile);
        }
    }
}

__attribute__((weak)) void pthreadpool_parallelize_2d(
    pthreadpool_t threadpool,
    pthreadpool_task_2d_t function,
    void* argument,
    size_t range_i,
    size_t range_j,
    size_t tile) {
    // Fallback to single-threaded execution
    if (function && range_i > 0 && range_j > 0) {
        for (size_t i = 0; i < range_i; i++) {
            for (size_t j = 0; j < range_j; j++) {
                function(argument, i, j);
            }
        }
    }
}

__attribute__((weak)) void pthreadpool_parallelize_2d_tile_1d(
    pthreadpool_t threadpool,
    pthreadpool_task_2d_tile_1d_t function,
    void* argument,
    size_t range_i,
    size_t range_j,
    size_t tile_j) {
    // Fallback to single-threaded execution
    if (function && range_i > 0 && range_j > 0) {
        size_t tile_size = (tile_j > 0) ? tile_j : 1;
        for (size_t i = 0; i < range_i; i++) {
            for (size_t j = 0; j < range_j; j += tile_size) {
                size_t actual_tile = (j + tile_size <= range_j) ? tile_size : (range_j - j);
                function(argument, i, j, actual_tile);
            }
        }
    }
}

__attribute__((weak)) void pthreadpool_parallelize_2d_tile_2d(
    pthreadpool_t threadpool,
    pthreadpool_task_2d_tile_2d_t function,
    void* argument,
    size_t range_i,
    size_t range_j,
    size_t tile_i,
    size_t tile_j) {
    // Fallback to single-threaded execution
    if (function && range_i > 0 && range_j > 0) {
        size_t tile_size_i = (tile_i > 0) ? tile_i : 1;
        size_t tile_size_j = (tile_j > 0) ? tile_j : 1;
        for (size_t i = 0; i < range_i; i += tile_size_i) {
            for (size_t j = 0; j < range_j; j += tile_size_j) {
                size_t actual_tile_i = (i + tile_size_i <= range_i) ? tile_size_i : (range_i - i);
                size_t actual_tile_j = (j + tile_size_j <= range_j) ? tile_size_j : (range_j - j);
                function(argument, i, j, actual_tile_i, actual_tile_j);
            }
        }
    }
}

__attribute__((weak)) size_t pthreadpool_get_threads_count(pthreadpool_t threadpool) {
    return 1; // Single-threaded fallback
}
"#;

    std::fs::write(&dest_path, stub_code).expect("Failed to write weak stub code");

    // Compile the stub code
    cc::Build::new().file(&dest_path).compile("weak_stubs");
}
