use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: export_schema <output-path>")?;

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &output,
        flowable_modeler_protocol::editor_protocol_schema_json()?,
    )?;
    println!("wrote {}", output.display());
    Ok(())
}
