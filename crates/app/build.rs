use std::{env, error::Error, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let version = env::var("CARGO_PKG_VERSION")?;
    let numeric = version.split(['-', '+']).next().unwrap_or(&version);
    let mut parts = numeric.split('.');
    let mut next_part = || parts.next().unwrap_or("0");
    let (major, minor, patch) = (next_part(), next_part(), next_part());

    let template = fs::read_to_string("resources/app.rc")?;
    let icon = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("resources/icon.ico");
    let resource = template
        .replace("@VERSION_NUMBERS@", &format!("{major},{minor},{patch},0"))
        .replace("@VERSION@", &version)
        .replace("@ICON@", &icon.display().to_string().replace('\\', "/"));
    let resource_path = PathBuf::from(env::var("OUT_DIR")?).join("app.rc");
    fs::write(&resource_path, resource)?;

    println!("cargo:rerun-if-changed=resources");
    embed_resource::compile(&resource_path, embed_resource::NONE)
        .manifest_optional()
        .map_err(|error| error.to_string())?;
    Ok(())
}
