use std::{env, error::Error, fs, path::PathBuf};

const THEMES: [&str; 7] = [
    "teal", "ocean", "mint", "amber", "coral", "purple", "graphite",
];

fn main() -> Result<(), Box<dyn Error>> {
    let version = env::var("CARGO_PKG_VERSION")?;
    let numeric = version.split(['-', '+']).next().unwrap_or(&version);
    let mut parts = numeric.split('.');
    let mut next_part = || parts.next().unwrap_or("0");
    let (major, minor, patch) = (next_part(), next_part(), next_part());

    let template = fs::read_to_string("resources/app.rc")?;
    let icons_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("resources/icons");
    // One icon per color theme, with resource IDs from 1 in `ColorTheme::ALL` order (a test
    // checks the two agree). The first is the exe's own icon in Explorer.
    let icons = THEMES
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let path = icons_dir.join(format!("{name}.ico"));
            format!(
                "{} ICON \"{}\"",
                index + 1,
                path.display().to_string().replace('\\', "/")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    println!("cargo:rustc-env=VIBEPLAYER_ICON_ORDER={}", THEMES.join(","));
    let resource = template
        .replace("@VERSION_NUMBERS@", &format!("{major},{minor},{patch},0"))
        .replace("@VERSION@", &version)
        .replace("@ICONS@", &icons);
    let resource_path = PathBuf::from(env::var("OUT_DIR")?).join("app.rc");
    fs::write(&resource_path, resource)?;

    println!("cargo:rerun-if-changed=resources");
    embed_resource::compile(&resource_path, embed_resource::NONE)
        .manifest_optional()
        .map_err(|error| error.to_string())?;
    Ok(())
}
