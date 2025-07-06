fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/audio.proto")?;

    // Ensure we rebuild when proto files change
    println!("cargo:rerun-if-changed=proto/audio.proto");

    Ok(())
}
