use std::path::PathBuf;

/// Resolve the platform-specific system Codex directory used for machine-wide config and skills.
pub fn system_codex_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        Some(PathBuf::from("/etc/codex"))
    }

    #[cfg(windows)]
    {
        Some(windows_codex_system_dir())
    }

    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

#[cfg(windows)]
const DEFAULT_PROGRAM_DATA_DIR_WINDOWS: &str = r"C:\ProgramData";

#[cfg(windows)]
fn windows_codex_system_dir() -> PathBuf {
    let program_data = windows_program_data_dir_from_known_folder().unwrap_or_else(|err| {
        tracing::warn!(
            error = %err,
            "Failed to resolve ProgramData known folder; using default path"
        );
        PathBuf::from(DEFAULT_PROGRAM_DATA_DIR_WINDOWS)
    });
    program_data.join("OpenAI").join("Codex")
}

#[cfg(windows)]
fn windows_program_data_dir_from_known_folder() -> std::io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::FOLDERID_ProgramData;
    use windows_sys::Win32::UI::Shell::KF_FLAG_DEFAULT;
    use windows_sys::Win32::UI::Shell::SHGetKnownFolderPath;

    let mut path_ptr = std::ptr::null_mut::<u16>();
    let known_folder_flags = u32::try_from(KF_FLAG_DEFAULT).map_err(|_| {
        std::io::Error::other(format!(
            "KF_FLAG_DEFAULT did not fit in u32: {KF_FLAG_DEFAULT}"
        ))
    })?;

    // SAFETY: SHGetKnownFolderPath initializes `path_ptr` with a CoTaskMem-allocated,
    // null-terminated UTF-16 string on success.
    let hr = unsafe {
        SHGetKnownFolderPath(&FOLDERID_ProgramData, known_folder_flags, 0, &mut path_ptr)
    };
    if hr != 0 {
        return Err(std::io::Error::other(format!(
            "SHGetKnownFolderPath(FOLDERID_ProgramData) failed with HRESULT {hr:#010x}"
        )));
    }
    if path_ptr.is_null() {
        return Err(std::io::Error::other(
            "SHGetKnownFolderPath(FOLDERID_ProgramData) returned a null pointer",
        ));
    }

    // SAFETY: `path_ptr` is a valid null-terminated UTF-16 string allocated by
    // SHGetKnownFolderPath and must be freed with CoTaskMemFree.
    let path = unsafe {
        let mut len = 0usize;
        while *path_ptr.add(len) != 0 {
            len += 1;
        }
        let wide = std::slice::from_raw_parts(path_ptr, len);
        let path = PathBuf::from(OsString::from_wide(wide));
        CoTaskMemFree(path_ptr.cast());
        path
    };

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::system_codex_dir;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    #[cfg(unix)]
    #[test]
    fn system_codex_dir_uses_etc_codex() {
        assert_eq!(system_codex_dir(), Some(PathBuf::from("/etc/codex")));
    }

    #[cfg(windows)]
    #[test]
    fn system_codex_dir_uses_expected_windows_suffix() {
        use std::path::Path;

        let expected = super::windows_program_data_dir_from_known_folder()
            .unwrap_or_else(|_| PathBuf::from(super::DEFAULT_PROGRAM_DATA_DIR_WINDOWS))
            .join("OpenAI")
            .join("Codex");

        assert_eq!(system_codex_dir(), Some(expected.clone()));
        assert!(expected.ends_with(Path::new("OpenAI").join("Codex")));
    }
}
