use cpal::traits::HostTrait;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/audio.proto")?;

    // Check if CPAL can find any audio devices
    let host = cpal::default_host();
    if let Ok(devices) = host.devices() {
        let device_count = devices.count();
        if device_count > 0 {
            println!("cargo:rustc-cfg=feature=\"audio_available\"");
            println!("cargo:rerun-if-changed=build.rs");
        }
    }

    Ok(())
}
