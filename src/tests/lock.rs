use super::*;

#[test]
#[ignore]
fn holds_a_lock_for_another_process_to_find() {
    let Ok(path) = std::env::var("BUILDC_TEST_LOCK") else {
        return;
    };
    let _held = hold_at(Path::new(&path)).expect("the child takes the lock");
    std::fs::write(format!("{path}.taken"), "").expect("the child reports it");
    std::thread::sleep(std::time::Duration::from_secs(30));
}

#[test]
fn a_second_installer_is_refused_and_told_where_the_first_one_is() {
    let path = std::env::temp_dir().join(format!("tect-lock-{}", std::process::id()));
    let taken = PathBuf::from(format!("{}.taken", path.display()));
    let _ = std::fs::remove_file(&taken);

    let mut child = Command::new(std::env::current_exe().expect("the test binary"))
        .args([
            "--ignored",
            "--exact",
            "tests::lock::holds_a_lock_for_another_process_to_find",
        ])
        .env("BUILDC_TEST_LOCK", &path)
        .stdout(Stdio::null())
        .spawn()
        .expect("the test binary re-runs itself as the holder");
    for _ in 0..200 {
        if taken.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(taken.exists(), "the holder never took the lock");

    let refused = hold_at(&path).expect_err("a second installer is refused");
    assert!(
        refused.starts_with("the installer is running on "),
        "{refused}"
    );
    assert!(refused.contains("again once it finishes"), "{refused}");
    child.kill().expect("the holder is killed");
    child.wait().expect("the holder is reaped");
    assert!(
        hold_at(&path).is_ok(),
        "the lock outlived the process holding it"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&taken);
}

#[test]
fn a_pty_holder_is_named_as_the_graphical_console() {
    assert_eq!(name(Some("/dev/ttyS0".into())), "/dev/ttyS0");
    assert_eq!(name(Some("/dev/tty1".into())), "/dev/tty1");
    assert_eq!(name(Some("/dev/console".into())), "/dev/console");
    assert_eq!(
        name(Some("/dev/pts/0".into())),
        "another session (/dev/pts/0)"
    );
    assert_eq!(name(Some("/dev/null".into())), "another console");
    assert_eq!(name(Some("/proc/1/fd/0".into())), "another console");
    assert_eq!(name(None), "another console");
    assert_eq!(console(-1), "another console");
}
