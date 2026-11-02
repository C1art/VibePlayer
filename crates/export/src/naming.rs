use std::path::{Path, PathBuf};

/// `clip.mp4` → `clip (trim).mp4`, next to the source.
pub fn suggested_output(source: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "video".into());
    let name = match source.extension() {
        Some(extension) => format!("{stem} (trim).{}", extension.to_string_lossy()),
        None => format!("{stem} (trim)"),
    };
    source.with_file_name(name)
}

/// `path`, or `name (2).ext`, `name (3).ext`… whichever doesn't exist yet.
pub fn unique_path(path: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    if !exists(path) {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|n| path.with_file_name(format!("{stem} ({n}){extension}")))
        .find(|candidate| !exists(candidate))
        .unwrap_or_else(|| path.to_path_buf())
}

/// A hidden sibling of `output` to write into before verification.
pub(crate) fn temp_path(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Keep the real extension last so FFmpeg still picks the right container.
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) => (stem.to_owned(), format!(".{extension}")),
        None => (name, String::new()),
    };
    output.with_file_name(format!(".{stem}.partial{extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestion_sits_next_to_the_source_with_its_extension() {
        assert_eq!(
            suggested_output(Path::new("/videos/clip.mkv")),
            PathBuf::from("/videos/clip (trim).mkv")
        );
    }

    #[test]
    fn existing_names_get_a_number() {
        let taken = [
            PathBuf::from("a (trim).mp4"),
            PathBuf::from("a (trim) (2).mp4"),
        ];
        let unique = unique_path(Path::new("a (trim).mp4"), |p| taken.iter().any(|t| t == p));
        assert_eq!(unique, PathBuf::from("a (trim) (3).mp4"));
    }

    #[test]
    fn temp_files_keep_the_container_extension() {
        assert_eq!(
            temp_path(Path::new("/v/out.mp4")),
            PathBuf::from("/v/.out.partial.mp4")
        );
    }
}
