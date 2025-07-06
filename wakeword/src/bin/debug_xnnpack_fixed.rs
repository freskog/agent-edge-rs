use env_logger;
use wakeword::xnnpack_fix::create_xnnpack_options;

fn main() {
    println!("=== XNNPACK Fixed Debug Binary ===");

    // Enable debug logging
    std::env::set_var("RUST_LOG", "debug");
    env_logger::init();

    println!("Step 1: Testing XNNPACK options creation with fixed wrapper...");

    // This should NOT crash, unlike the broken tflitec version
    let xnnpack_options = create_xnnpack_options(1);
    println!("✅ XNNPACK options created successfully with fixed wrapper!");

    println!("Thread count: {}", xnnpack_options.num_threads);

    println!("Step 2: Testing XNNPACK delegate creation...");

    // Create interpreter options using our wrapper
    let interpreter_options_ptr = unsafe { wakeword::xnnpack_fix::create_interpreter_options() };

    if interpreter_options_ptr.is_null() {
        panic!("Failed to create interpreter options");
    }

    // Set thread count using our wrapper
    unsafe {
        wakeword::xnnpack_fix::set_thread_count(interpreter_options_ptr, 1);
    }

    println!("Creating XNNPACK delegate with fixed wrapper...");
    let delegate_ptr = unsafe {
        wakeword::xnnpack_fix::create_xnnpack_delegate(&xnnpack_options, interpreter_options_ptr)
    };

    if delegate_ptr.is_null() {
        panic!("Failed to create XNNPACK delegate");
    }

    println!("✅ XNNPACK delegate created successfully!");

    // Cleanup using our wrappers
    unsafe {
        wakeword::xnnpack_fix::delete_xnnpack_delegate(delegate_ptr);
        wakeword::xnnpack_fix::delete_interpreter_options(interpreter_options_ptr);
    }

    println!("🎉 XNNPACK C ABI fix is working correctly!");
    println!("The segfault in TfLiteXNNPackDelegateOptionsDefault() has been resolved!");
}
