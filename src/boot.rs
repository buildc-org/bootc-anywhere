use super::*;

const SIGNED_MARKER: &str = "/usr/share/secureboot/signed";

const RENDERER: &str = "/usr/libexec/grub-menu-from-bls";

const TARGET: &str = "/run/buildc-target";

pub(crate) fn configure_boot_chain(image: &str, disk: &str, chain: &str) -> Result<(), String> {
    if chain.is_empty() {
        return Ok(());
    }
    let at = PathBuf::from(TARGET);
    let boot = at.join("boot");
    std::fs::create_dir_all(&boot).map_err(|err| format!("{TARGET}: {err}"))?;
    let Some((device, root)) = boot_partition(disk, &boot)? else {
        return Err(format!(
            "no partition of {disk} carries `loader/entries`, so the {chain} chain cannot be configured"
        ));
    };
    let esp = match root {
        "/target" => boot.clone(),
        _ => boot.join("boot"),
    };

    let configured = match chain {
        "uki-db" => {
            let missing = ["PK", "KEK", "db"]
                .into_iter()
                .map(|name| esp.join(format!("loader/keys/auto/{name}.auth")))
                .find(|key| !key.is_file());
            match missing {
                Some(key) => Err(format!(
                    "{}: bootc did not install the {chain} enrollment key",
                    key.display()
                )),
                None => enroll_force(&esp.join("loader/loader.conf")),
            }
        }
        "uki-shim" => run_enrolment(image, &esp),
        _ => Err(format!("unknown boot chain `{chain}`")),
    };
    let unmounted = unmount(&device, &boot);
    configured?;
    unmounted?;
    eprintln!("{PROGRAM}: configured {chain} on {device}");
    Ok(())
}

pub(crate) fn enroll_force(loader: &Path) -> Result<(), String> {
    let current = read_held(loader)?;
    let mut lines: Vec<&str> = current
        .lines()
        .filter(|line| !line.trim_start().starts_with("secure-boot-enroll "))
        .collect();
    lines.push("secure-boot-enroll force");
    std::fs::write(loader, format!("{}\n", lines.join("\n")))
        .map_err(|err| format!("{}: {err}", loader.display()))
}

fn unmount(device: &str, at: &Path) -> Result<(), String> {
    match Command::new("umount").arg(at).output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!(
            "unmounting {device}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(err) => Err(format!("unmounting {device}: {err}")),
    }
}

pub(crate) fn require_signed_boot_chain(image: &str, chain: &str) -> Result<(), String> {
    if chain.is_empty() {
        return Ok(());
    }
    let out = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--net=none",
            "--security-opt",
            "label=disable",
            "--entrypoint",
            "/usr/bin/test",
            image,
            "-e",
            SIGNED_MARKER,
        ])
        .output()
        .map_err(|err| format!("podman: {err}, and it is what verifies the {chain} chain"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "{image} does not carry {SIGNED_MARKER}, which its declared {chain} chain needs; \
         build it with the Secure Boot private key, or with a `boot/uki` new enough to write \
         that path, before installing"
    ))
}

pub(crate) fn require_tpm2_enrolment(image: &str) -> Result<(), String> {
    let out = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--net=none",
            "--security-opt",
            "label=disable",
            "--entrypoint",
            "",
            image,
            "/usr/bin/systemd-cryptenroll",
            "--version",
        ])
        .output()
        .map_err(|err| format!("podman: {err}, and it is what checks {image} for TPM2"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "{image} has no /usr/bin/systemd-cryptenroll, so it cannot enrol the TPM2 token this answer asks for"
    ))
}

pub(crate) fn pcr_policy_in(image: &str) -> bool {
    Command::new("podman")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--net=none",
            "--security-opt",
            "label=disable",
            "--entrypoint",
            "",
            image,
            "/usr/bin/test",
            "-s",
            "/usr/share/secureboot/pcr-policy.pem",
        ])
        .output()
        .is_ok_and(|out| out.status.success())
}

fn run_enrolment(image: &str, esp: &Path) -> Result<(), String> {
    let target = format!("{}:/esp", esp.display());
    let out = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--net=none",
            "--security-opt",
            "label=disable",
            "--entrypoint",
            "",
            "-v",
            &target,
            image,
            "/usr/libexec/secureboot-enrolment",
            "/esp",
        ])
        .output()
        .map_err(|err| format!("podman: {err}, and it is what installs the shim chain"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "/usr/libexec/secureboot-enrolment in {image}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

pub(crate) fn render_menu(image: &str, disk: &str) -> Result<(), String> {
    let at = PathBuf::from(TARGET);
    let boot = at.join("boot");
    std::fs::create_dir_all(&boot).map_err(|err| format!("{TARGET}: {err}"))?;

    let Some((device, root)) = boot_partition(disk, &boot)? else {
        return Err(format!(
            "no partition of {disk} carries `loader/entries`, so there is no menu to render"
        ));
    };
    let rendered = run_renderer(image, root);
    let unmounted = unmount(&device, &boot);
    let rendered = rendered.and_then(|wrote| unmounted.map(|()| wrote));
    match rendered {
        Ok(true) => {
            eprintln!("{PROGRAM}: wrote the boot menu {device} needs, since its GRUB reads no BLS");
            Ok(())
        }
        Ok(false) => Ok(()),
        Err(err) => Err(err),
    }
}

fn boot_partition(disk: &str, boot: &Path) -> Result<Option<(String, &'static str)>, String> {
    let listed = Command::new("lsblk")
        .args(["-nrpo", "NAME", disk])
        .output()
        .map_err(|err| format!("lsblk: {err}, and it is what lists a disk's partitions"))?;
    for device in labelled(&String::from_utf8_lossy(&listed.stdout)) {
        if device == disk {
            continue;
        }
        let mounted = Command::new("mount")
            .args([&device, &boot.to_string_lossy().to_string()])
            .output();
        if !matches!(&mounted, Ok(out) if out.status.success()) {
            continue;
        }
        if boot.join("loader/entries").is_dir() || boot.join("EFI/Linux").is_dir() {
            return Ok(Some((device, "/target")));
        }
        if boot.join("boot/loader/entries").is_dir() || boot.join("boot/EFI/Linux").is_dir() {
            return Ok(Some((device, "/target/boot")));
        }
        unmount(&device, boot)?;
    }
    Ok(None)
}

fn run_renderer(image: &str, root: &str) -> Result<bool, String> {
    let out = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--net=none",
            "--security-opt",
            "label=disable",
            "--entrypoint",
            "",
            "-v",
            &format!("{TARGET}:/target"),
            image,
            "/bin/sh",
            "-c",
            &format!("test -x {RENDERER} || exit 3; exec {RENDERER} {root}"),
        ])
        .output()
        .map_err(|err| format!("podman: {err}, and it is what runs the image's renderer"))?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(3) => Ok(false),
        _ => Err(format!(
            "{RENDERER} in {image}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}
