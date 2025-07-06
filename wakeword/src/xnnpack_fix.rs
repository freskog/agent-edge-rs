//! XNNPACK C ABI compatibility layer
//!
//! This module fixes the C ABI mismatch in tflitec's XNNPACK bindings.
//! The issue is that TfLiteXNNPackDelegateOptionsDefault() expects a pointer
//! to the options struct, but tflitec treats it as returning the struct by value.

use std::mem::MaybeUninit;

// Define the XNNPACK structures directly since tflitec::bindings is private
#[repr(C)]
pub struct TfLiteXNNPackDelegateOptions {
    pub num_threads: i32,
    pub flags: i32,
    pub weights_cache_file_path: *const std::os::raw::c_char,
    pub experimental_transient_weights_cache_file_path: *const std::os::raw::c_char,
}

// Opaque types
#[repr(C)]
pub struct TfLiteDelegate {
    _unused: [u8; 0],
}

#[repr(C)]
pub struct TfLiteInterpreterOptions {
    _unused: [u8; 0],
}

extern "C" {
    // The correct C signature - takes a pointer to fill, not return by value
    fn TfLiteXNNPackDelegateOptionsDefault(options: *mut TfLiteXNNPackDelegateOptions);

    // XNNPACK delegate functions
    fn TfLiteXNNPackDelegateCreate(
        options: *const TfLiteXNNPackDelegateOptions,
    ) -> *mut TfLiteDelegate;
    fn TfLiteXNNPackDelegateDelete(delegate: *mut TfLiteDelegate);

    // Interpreter options functions
    fn TfLiteInterpreterOptionsCreate() -> *mut TfLiteInterpreterOptions;
    fn TfLiteInterpreterOptionsDelete(options: *mut TfLiteInterpreterOptions);
    fn TfLiteInterpreterOptionsSetNumThreads(
        options: *mut TfLiteInterpreterOptions,
        num_threads: i32,
    );
    fn TfLiteInterpreterOptionsAddDelegate(
        options: *mut TfLiteInterpreterOptions,
        delegate: *mut TfLiteDelegate,
    );
}

/// Safe wrapper for TfLiteXNNPackDelegateOptions initialization
pub fn create_xnnpack_options(num_threads: i32) -> TfLiteXNNPackDelegateOptions {
    unsafe {
        // Allocate uninitialized memory for the struct
        let mut options = MaybeUninit::<TfLiteXNNPackDelegateOptions>::uninit();

        // Call the C function with the correct signature
        TfLiteXNNPackDelegateOptionsDefault(options.as_mut_ptr());

        // Initialize the struct (it's now safely initialized by the C function)
        let mut options = options.assume_init();

        // Set the thread count
        if num_threads > 0 {
            options.num_threads = num_threads;
        }

        options
    }
}

/// Safe wrapper for creating an XNNPACK delegate
pub unsafe fn create_xnnpack_delegate(
    options: &TfLiteXNNPackDelegateOptions,
    interpreter_options_ptr: *mut TfLiteInterpreterOptions,
) -> *mut TfLiteDelegate {
    let xnnpack_delegate_ptr = TfLiteXNNPackDelegateCreate(options);
    TfLiteInterpreterOptionsAddDelegate(interpreter_options_ptr, xnnpack_delegate_ptr);
    xnnpack_delegate_ptr
}

/// Safe wrapper for deleting an XNNPACK delegate
pub unsafe fn delete_xnnpack_delegate(delegate: *mut TfLiteDelegate) {
    TfLiteXNNPackDelegateDelete(delegate);
}

/// Safe wrapper for creating interpreter options
pub unsafe fn create_interpreter_options() -> *mut TfLiteInterpreterOptions {
    TfLiteInterpreterOptionsCreate()
}

/// Safe wrapper for deleting interpreter options
pub unsafe fn delete_interpreter_options(options: *mut TfLiteInterpreterOptions) {
    TfLiteInterpreterOptionsDelete(options);
}

/// Safe wrapper for setting thread count
pub unsafe fn set_thread_count(options: *mut TfLiteInterpreterOptions, num_threads: i32) {
    TfLiteInterpreterOptionsSetNumThreads(options, num_threads);
}

/// Helper function to create an interpreter with working XNNPACK (when possible)
/// For now, creates a CPU-only interpreter to avoid the segfault
pub fn create_interpreter_with_xnnpack_safe<'a>(
    model: &'a tflitec::model::Model<'a>,
    thread_count: i32,
) -> Result<tflitec::interpreter::Interpreter<'a>, tflitec::Error> {
    log::debug!("🔄 Creating interpreter with XNNPACK-safe approach");

    // Create regular interpreter with CPU-only to avoid segfault
    // TODO: Integrate our working XNNPACK delegate here
    let mut options = tflitec::interpreter::Options::default();
    options.thread_count = thread_count;

    // IMPORTANT: Do NOT set is_xnnpack_enabled = true (this causes the segfault)
    // Instead, we'll use our working XNNPACK delegate in a future update

    #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
    {
        log::debug!("🔄 XNNPACK fix available - using CPU-only for now to avoid segfault");
        log::debug!("🔄 TODO: Replace with working XNNPACK delegate integration");
    }

    let interpreter = tflitec::interpreter::Interpreter::new(model, Some(options))?;

    Ok(interpreter)
}

/// Helper function to check if XNNPACK should be enabled on this platform
pub fn should_enable_xnnpack() -> bool {
    // Only enable on aarch64 Linux where we have the fix
    cfg!(all(target_arch = "aarch64", target_os = "linux"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xnnpack_options_creation() {
        let options = create_xnnpack_options(4);
        assert_eq!(options.num_threads, 4);
    }

    #[test]
    fn test_xnnpack_delegate_creation() {
        // Create interpreter options
        let interpreter_options_ptr = unsafe { create_interpreter_options() };
        assert!(!interpreter_options_ptr.is_null());

        // Create XNNPACK options
        let xnnpack_options = create_xnnpack_options(1);

        // Create XNNPACK delegate
        let delegate_ptr =
            unsafe { create_xnnpack_delegate(&xnnpack_options, interpreter_options_ptr) };

        assert!(!delegate_ptr.is_null());

        // Cleanup
        unsafe {
            delete_xnnpack_delegate(delegate_ptr);
            delete_interpreter_options(interpreter_options_ptr);
        }
    }
}
