use super::*;

#[test]
fn worker_arguments_require_loopback_and_strong_token() {
    let token = "ab".repeat(32);
    let parsed = WorkerArguments::from_parts("127.0.0.1:1234", token.clone()).unwrap();
    assert_eq!(parsed.endpoint, "127.0.0.1:1234".parse().unwrap());
    assert_eq!(parsed.token, token);

    assert!(WorkerArguments::from_parts("192.0.2.1:1234", "ab".repeat(32)).is_err());
}

#[test]
fn protocol_rejects_unknown_commands() {
    assert!(serde_json::from_str::<WorkerCommand>(r#"{"command":"shell"}"#).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn flatpak_worker_uses_pinned_app_and_runtime_without_a_shell() {
    use std::ffi::OsString;
    use std::io::Write as _;

    let mut metadata = tempfile::NamedTempFile::new().unwrap();
    metadata
        .write_all(
            b"[Instance]\n\
              app-path=/var/lib/flatpak/app/io.github.ergolyam.Drosophila/commit/files\n\
              runtime-path=/var/lib/flatpak/runtime/org.gnome.Platform/commit/files\n",
        )
        .unwrap();
    let arguments = vec![
        WORKER_FLAG.to_owned(),
        "127.0.0.1:1234".to_owned(),
        "$(touch /tmp/not-a-command)".to_owned(),
    ];

    let launch = LinuxLaunch::elevated_flatpak(&arguments, metadata.path()).unwrap();

    assert_eq!(launch.executable, PathBuf::from("flatpak-spawn"));
    assert_eq!(launch.arguments[0], OsString::from("--host"));
    assert_eq!(launch.arguments[1], OsString::from("pkexec"));
    let (loader_name, linux_triplet) = flatpak_linux_abi().unwrap();
    assert_eq!(
        Path::new(&launch.arguments[3]),
        Path::new("/var/lib/flatpak/runtime/org.gnome.Platform/commit/files")
            .join("lib")
            .join(linux_triplet)
            .join(loader_name)
    );
    assert!(
        !launch
            .arguments
            .iter()
            .any(|argument| argument == "/bin/sh" || argument == "-c")
    );
    assert_eq!(
        launch.arguments.last(),
        Some(&OsString::from("$(touch /tmp/not-a-command)"))
    );
    assert!(launch.arguments.iter().any(|argument| {
        Path::new(argument)
            == Path::new(
                "/var/lib/flatpak/app/io.github.ergolyam.Drosophila/commit/files/bin/drosophila",
            )
    }));
}

#[cfg(target_os = "linux")]
#[test]
fn flatpak_worker_rejects_non_absolute_deployment_paths() {
    use std::io::Write as _;

    let mut metadata = tempfile::NamedTempFile::new().unwrap();
    metadata
        .write_all(b"[Instance]\napp-path=relative\nruntime-path=/runtime\n")
        .unwrap();

    assert!(LinuxLaunch::elevated_flatpak(&[], metadata.path()).is_err());
}
