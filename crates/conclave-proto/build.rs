fn main() -> Result<(), Box<dyn std::error::Error>> {
    prost_build::compile_protos(
        &["../../proto/conclave/v1/conclave.proto"],
        &["../../proto/"],
    )?;
    Ok(())
}
