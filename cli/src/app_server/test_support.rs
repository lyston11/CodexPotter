use std::path::Path;

#[cfg(any(unix, windows))]
pub fn write_dummy_codex_script(path: &Path, script: impl AsRef<str>) {
    use std::io::Write as _;

    let script = script.as_ref();
    let parent = path.parent().expect("dummy codex path should have parent");
    let mut tmp = tempfile::NamedTempFile::new_in(parent).expect("create dummy codex temp");

    // Write and chmod the temp file before persisting it into place. This avoids intermittently
    // spawning a script that is still being written or whose executable bit is not visible yet
    // under parallel test load.
    tmp.write_all(script.as_bytes()).expect("write dummy codex");
    if !script.ends_with('\n') {
        tmp.write_all(b"\n")
            .expect("write dummy codex trailing newline");
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = tmp
            .as_file()
            .metadata()
            .expect("stat dummy codex")
            .permissions();
        perms.set_mode(0o755);
        tmp.as_file()
            .set_permissions(perms)
            .expect("chmod dummy codex");
    }

    tmp.as_file().sync_all().expect("sync dummy codex");

    tmp.persist(path)
        .map_err(|err| err.error)
        .expect("persist dummy codex");
}

#[cfg(any(unix, windows))]
pub async fn lock_dummy_codex_test() -> tokio::sync::MutexGuard<'static, ()> {
    static DUMMY_CODEX_TEST_MUTEX: std::sync::OnceLock<tokio::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    // These tests spawn shell-backed dummy `codex` processes and assert on timed async event
    // sequences. Running them concurrently across `app_server` modules causes resource contention
    // and sporadic missed-event failures under `cargo test -p`.
    DUMMY_CODEX_TEST_MUTEX
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}
