#![expect(
    clippy::unwrap_used,
    reason = "a test helper panics on a failed step, which fails the calling test"
)]

use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tmp() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn empty(name: &str) -> PathBuf {
    let dir = tmp().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn terminal_flow(dir: &Path, command: &str, steps: &[(&[u8], &[u8])]) -> Vec<u8> {
    use std::io::{Read, Write};
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let mut child = std::process::Command::new("script")
        .args(["-qfec", command, "/dev/null"])
        .current_dir(dir)
        .env("COLUMNS", "80")
        .env_remove("NO_COLOR")
        .env("BUILDC_TPM", "/nonexistent")
        .env("BUILDC_CARET", "static")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("script from util-linux");
    let input = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let mut output = child.stdout.take().unwrap();
    let raw = Arc::new(Mutex::new(Vec::new()));
    let reader = {
        let (input, raw) = (input.clone(), raw.clone());
        std::thread::spawn(move || {
            let mut byte = [0];
            while output.read_exact(&mut byte).is_ok() {
                let mut held = raw.lock().unwrap();
                held.push(byte[0]);
                // The terminal UI waits for a cursor position report, so the reader answers every query.
                if held.ends_with(b"\x1b[6n") {
                    let mut input = input.lock().unwrap();
                    let _ = input.write_all(b"\x1b[1;1R");
                    let _ = input.flush();
                }
            }
        })
    };
    for (marker, keys) in steps {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let seen = raw
                .lock()
                .unwrap()
                .windows(marker.len())
                .any(|window| window == *marker);
            if seen {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the terminal did not draw {:?}: {}",
                String::from_utf8_lossy(marker),
                String::from_utf8_lossy(&raw.lock().unwrap())
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut input = input.lock().unwrap();
        input.write_all(keys).unwrap();
        input.flush().unwrap();
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let status = loop {
        match child.try_wait().unwrap() {
            Some(status) => break status,
            None if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50))
            }
            None => {
                child
                    .kill()
                    .expect("the installer is killed at the deadline");
                break child.wait().unwrap();
            }
        }
    };
    reader.join().unwrap();
    let mut errors = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut errors)
        .unwrap();
    let raw = raw.lock().unwrap();
    assert!(
        status.success(),
        "{errors}{}",
        String::from_utf8_lossy(&raw)
    );

    raw.clone()
}

#[test]
fn version_names_the_program_and_the_release() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_bootc-anywhere"))
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{} v{}\n", installer::PROGRAM, env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn a_bad_invocation_is_refused_with_the_usage_code() {
    for (args, said) in [
        (["--bogus"].as_slice(), "unexpected argument"),
        (["--disk"].as_slice(), "a value is required"),
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_bootc-anywhere"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.starts_with("Error: "), "{args:?}: {stderr}");
        assert!(stderr.contains(said), "{args:?}: {stderr}");
    }
}

#[test]
fn help_prints_every_flag_and_exits_zero() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_bootc-anywhere"))
        .arg("--help")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    for flag in [
        "--from",
        "--disk",
        "--hostname",
        "--user",
        "--password",
        "--encryption",
        "--passphrase",
        "--pin",
        "--no-tui",
    ] {
        assert!(stdout.contains(flag), "{flag} is not in the help: {stdout}");
    }
}

#[test]
fn commands_doc() {
    use clap::CommandFactory;
    let path = crate_dir().join("docs/commands.md");
    let options = clap_markdown::MarkdownOptions::new()
        .title("Commands".to_string())
        .show_footer(false);
    let rendered =
        clap_markdown::help_markdown_command_custom(&installer::cli::Args::command(), &options);
    let doc = std::fs::read_to_string(&path).expect("docs/commands.md exists");
    assert!(doc == rendered, "docs/commands.md is stale");
}

#[test]
fn a_second_installer_is_refused_while_the_first_holds_the_screen() {
    use std::time::Duration;

    let dir = empty("flow-install-lock");
    std::fs::write(
        dir.join(installer::RECIPE),
        r#"{
  "image": "ghcr.io/buildc-org/deb2:latest",
  "targetImgref": "ghcr.io/buildc-org/deb2:latest",
  "composeFsBackend": true,
  "genericImage": true,
  "bootloader": "grub2",
  "filesystem": "ext4",
  "luksInitramfs": true,
  "hostname": "deb2",
  "user": { "groups": ["sudo"] },
  "additionalImageStores": ["/var/lib/buildc/store"]
}
"#,
    )
    .unwrap();
    let lock = dir.join("installer.lock");
    let second = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_bootc-anywhere"))
            .args(["--from", "."])
            .current_dir(&dir)
            .env("BUILDC_INSTALLER_LOCK", &lock)
            .env("BUILDC_TPM", "/nonexistent")
            .output()
            .unwrap()
    };

    let mut first = std::process::Command::new("script")
        .args([
            "-qfec",
            &format!(
                "BUILDC_INSTALLER_LOCK='{}' '{}' --from .",
                lock.display(),
                env!("CARGO_BIN_EXE_bootc-anywhere")
            ),
            "/dev/null",
        ])
        .current_dir(&dir)
        .env("COLUMNS", "80")
        .env("BUILDC_TPM", "/nonexistent")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("script from util-linux");

    let mut refused = None;
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(50));
        let out = second();
        let said = String::from_utf8_lossy(&out.stderr).into_owned();
        if said.contains("the installer is running on") {
            refused = Some(said);
            break;
        }
    }
    let killed = first.kill().and_then(|()| first.wait());
    let refused = refused.expect("the first installer never held the lock while it ran");
    killed.expect("the first installer is reaped");

    assert!(
        refused.contains(&format!("run `{}` again", installer::PROGRAM)),
        "{refused}"
    );
    let after = second();
    assert!(
        !String::from_utf8_lossy(&after.stderr).contains("the installer is running on"),
        "the lock outlived the process holding it: {}",
        String::from_utf8_lossy(&after.stderr)
    );
}

#[test]
fn installer_terminal_smoke_waits_for_each_screen() {
    use std::os::unix::fs::PermissionsExt;

    let dir = empty("flow-install-drawn");
    std::fs::write(
        dir.join(installer::RECIPE),
        r#"{
  "image": "ghcr.io/buildc-org/deb2:latest",
  "targetImgref": "ghcr.io/buildc-org/deb2:latest",
  "composeFsBackend": true,
  "genericImage": true,
  "bootloader": "grub2",
  "filesystem": "ext4",
  "luksInitramfs": true,
  "hostname": "deb2",
  "user": { "groups": ["sudo"] },
  "additionalImageStores": ["/var/lib/buildc/store"]
}
"#,
    )
    .unwrap();
    let sys = dir.join("sys-block");
    for (name, size, removable, model) in [
        ("vda", "134217728", "0", "QEMU HARDDISK"),
        ("sdb", "31457280", "1", "Cruzer Blade"),
    ] {
        let disk = sys.join(name);
        std::fs::create_dir_all(disk.join("device")).unwrap();
        std::fs::write(disk.join("size"), size).unwrap();
        std::fs::write(disk.join("removable"), removable).unwrap();
        std::fs::write(disk.join("device/model"), model).unwrap();
    }
    let lsblk = dir.join("lsblk");
    std::fs::write(
        &lsblk,
        r#"#!/bin/sh
case "$*" in
*--json*"/dev/sdb"*) printf '%s\n' '{"blockdevices":[{"name":"/dev/sdb","type":"disk","children":[]}]}' ;;
*--json*) printf '%s\n' '{"blockdevices":[{"name":"/dev/vda","type":"disk","children":[{"name":"/dev/vda1","size":"512M","fstype":"vfat","label":"EFI","type":"part","parttype":"C12A7328-F81F-11D2-BA4B-00A0C93EC93B"},{"name":"/dev/vda2","size":"63.5G","fstype":"ext4","label":"old-root","type":"part","parttype":null},{"name":"/dev/vda3","size":"60G","fstype":"crypto_LUKS","label":"","type":"part","parttype":null}]}]}' ;;
*SIZE*) printf '%s\n' '64G' ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&lsblk, std::fs::Permissions::from_mode(0o755)).unwrap();
    let dev = dir.join("dev");
    std::fs::create_dir_all(&dev).unwrap();
    let image = dev.join("vda");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(64 * 1024 * 1024 * 1024))
        .unwrap();
    let disk = image.to_string_lossy().to_string();
    let mut child = std::process::Command::new("sfdisk")
        .args(["-q", &disk])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("sfdisk from util-linux writes the fixture table");
    {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"label: gpt\nsize=512M, type=U\nsize=10G\nsize=10G\n")
            .unwrap();
    }
    assert!(child.wait().unwrap().success());
    let other = dev.join("sdb");
    std::fs::File::create(&other)
        .and_then(|file| file.set_len(16 * 1024 * 1024 * 1024))
        .unwrap();
    let other_disk = other.to_string_lossy().to_string();
    let mut child = std::process::Command::new("sfdisk")
        .args(["-q", &other_disk])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("sfdisk from util-linux writes the fixture table");
    {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"label: gpt\n")
            .unwrap();
    }
    assert!(child.wait().unwrap().success());
    let fake_mount = dir.join("mount");
    std::fs::write(
        &fake_mount,
        r#"#!/bin/sh
# `mount_ro` passes the device third and the mount point last.
case "$3" in
/dev/vda1) mkdir -p "$4/EFI/fedora" ;;
/dev/vda2)
    mkdir -p "$4/etc"
    printf 'PRETTY_NAME="Test OS"\n' > "$4/etc/os-release"
    ;;
esac
exit 0
"#,
    )
    .unwrap();
    std::fs::set_permissions(&fake_mount, std::fs::Permissions::from_mode(0o755)).unwrap();
    let fake_umount = dir.join("umount");
    std::fs::write(&fake_umount, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake_umount, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cryptsetup = dir.join("cryptsetup");
    std::fs::write(
        &cryptsetup,
        r#"#!/bin/sh
case "$1" in
luksDump) printf '%s\n' '{"keyslots":{"0":{}},"tokens":{}}' ;;
*) exit 1 ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&cryptsetup, std::fs::Permissions::from_mode(0o755)).unwrap();
    let podman = dir.join("podman");
    std::fs::write(
        &podman,
        r#"#!/bin/sh
case "$*" in
*"/usr/lib/efi"*) printf '%s\n' /usr/lib/efi/grub2/1/EFI/fedora /usr/lib/efi/shim/1/EFI/BOOT ;;
*) exit 1 ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&podman, std::fs::Permissions::from_mode(0o755)).unwrap();
    let efivars = dir.join("efivars");
    std::fs::create_dir_all(&efivars).unwrap();
    for (name, value) in [("SecureBoot", 0u8), ("SetupMode", 1u8)] {
        let mut bytes = vec![0, 0, 0, 7];
        bytes.push(value);
        std::fs::write(
            efivars.join(format!("{name}-8be4df61-93ca-11d2-aa0d-00e098032b8c")),
            bytes,
        )
        .unwrap();
    }
    let active = dir.join("tty0-active");
    std::fs::write(&active, "tty2\n").unwrap();
    let transcript = terminal_flow(
        &dir,
        &format!(
            "stty rows 50 cols 80; PATH='{}':\"$PATH\" BUILDC_INSTALLER_LOCK='{}' BUILDC_SYS_BLOCK='{}' BUILDC_DEV='{}' BUILDC_MOUNT_ROOT='{}' BUILDC_EFIVARS='{}' BUILDC_TTY0_ACTIVE='{}' '{}' --from .",
            dir.display(),
            dir.join("installer.lock").display(),
            sys.display(),
            dev.display(),
            dir.join("mounts").display(),
            efivars.display(),
            active.display(),
            env!("CARGO_BIN_EXE_bootc-anywhere")
        ),
        &[
            (b"hostname".as_slice(), b"\x1b".as_slice()),
            (
                b"Leave the installer?".as_slice(),
                b"\x1b[B\x1b[B\r".as_slice(),
            ),
        ],
    );
    let transcript = String::from_utf8_lossy(&transcript);
    assert!(transcript.contains("hostname"), "{transcript}");
    assert!(transcript.contains("Leave the installer?"), "{transcript}");
}
