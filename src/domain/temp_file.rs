use std::path::Path;

/// Check if a file path corresponds to a known temporary file pattern.
///
/// Only examines the filename (final component of the path), not the
/// directory components. Temp files can appear in any strand directory.
///
/// Currently detects:
/// - macOS `sed -i` temporary files: `sed` followed by exactly 7 characters
///   (e.g. `sedXXXXXXX`)
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use knot::domain::temp_file::is_known_temp_file;
///
/// assert!(is_known_temp_file(Path::new("/project/src/sedXXXXXXX")));
/// assert!(!is_known_temp_file(Path::new("/project/src/main.rs")));
/// ```
pub fn is_known_temp_file(path: &Path) -> bool {
    let filename = match path.file_name().and_then(|f| f.to_str()) {
        Some(name) => name,
        None => return false,
    };

    is_sed_temp_file(filename)
}

/// Check if a filename matches the macOS `sed -i` temp file pattern.
///
/// macOS `sed -i` creates a temporary file named `sed` followed by 7
/// random characters (e.g. `sedAbCdEfG`), then renames it to the target.
fn is_sed_temp_file(filename: &str) -> bool {
    filename.len() == 10 && filename.starts_with("sed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sed_temp_file_with_exact_7_chars_matches() {
        assert!(is_known_temp_file(Path::new("/project/src/sedXXXXXXX")));
        assert!(is_known_temp_file(Path::new("/project/src/sedAbCdEfG")));
        assert!(is_known_temp_file(Path::new("/project/src/sed1234567")));
        assert!(is_known_temp_file(Path::new("/project/src/sed_abcde1")));
    }

    #[test]
    fn sed_temp_file_in_nested_path_matches() {
        // Temp files can appear in any directory
        assert!(is_known_temp_file(Path::new(
            "/project/loom/strands/deep/nested/sedXXXXXXX"
        )));
    }

    #[test]
    fn sed_temp_file_bare_filename_matches() {
        // No directory prefix
        assert!(is_known_temp_file(Path::new("sedXXXXXXX")));
    }

    #[test]
    fn sed_temp_file_wrong_length_does_not_match() {
        // Just "sed" alone (3 total)
        assert!(!is_known_temp_file(Path::new("sed")));
        // sed with 4 chars (7 total) — too short
        assert!(!is_known_temp_file(Path::new("sedXXXX")));
        // sed with 5 chars (8 total) — too short
        assert!(!is_known_temp_file(Path::new("sedXXXXX")));
        // sed with 6 chars (9 total) — too short
        assert!(!is_known_temp_file(Path::new("sedXXXXXX")));
        // sed with 8 chars (11 total) — too long
        assert!(!is_known_temp_file(Path::new("sedXXXXXXXX")));
        // sed with 9 chars (12 total) — too long
        assert!(!is_known_temp_file(Path::new("sedXXXXXXXXX")));
    }

    #[test]
    fn non_sed_files_do_not_match() {
        assert!(!is_known_temp_file(Path::new("/project/src/main.rs")));
        assert!(!is_known_temp_file(Path::new("/project/docs/readme.md")));
        assert!(!is_known_temp_file(Path::new("/project/Cargo.toml")));
    }

    #[test]
    fn filenames_starting_with_sed_but_not_temp_pattern_do_not_match() {
        // Longer names starting with "sed" — too many characters
        assert!(!is_known_temp_file(Path::new("/project/src/sed_commands.md")));
        assert!(!is_known_temp_file(Path::new("/project/src/sedutils_tool")));
    }

    #[test]
    fn path_components_are_ignored_only_filename_checked() {
        // A directory named "sedXXXXXXX" should not match
        assert!(!is_known_temp_file(Path::new("/sedXXXXXXX/src/main.rs")));
        // A parent directory containing "sed" is irrelevant
        assert!(!is_known_temp_file(Path::new("/sed_tools/myfile.txt")));
    }

    #[test]
    fn path_with_no_filename_returns_false() {
        // Root path has no file name
        assert!(!is_known_temp_file(Path::new("/")));
        // Empty path
        assert!(!is_known_temp_file(Path::new("")));
    }
}
