use super::*;

pub(crate) fn panel(payload: &Payload) -> Vec<common::ui::HeaderLine> {
    panel_lines(&firmware(payload), payload)
}

pub(crate) fn firmware(payload: &Payload) -> crate::firmware::Firmware {
    let mut firmware = crate::firmware::Firmware::read(&efivars(), None);
    if firmware.platform_key == crate::firmware::PlatformKey::Other && !payload.boot.is_empty() {
        let owner = owner_certificate(&payload.image);
        firmware = crate::firmware::Firmware::read(&efivars(), owner.as_deref());
    }
    firmware
}

pub(crate) fn next_steps(boot: &str, firmware: &crate::firmware::Firmware) -> Option<&'static str> {
    use crate::firmware::PlatformKey;
    if !firmware.efivars {
        return None;
    }
    match boot {
        "uki-shim" => Some(copy::shim_steps()),
        "uki-db" => match (firmware.platform_key, firmware.database) {
            (PlatformKey::Owner, true) | (PlatformKey::None, _) => None,
            _ => Some(copy::next_steps_setup()),
        },
        _ => None,
    }
}

pub(crate) fn erase_warning(boot: &str, firmware: &crate::firmware::Firmware) -> Vec<&'static str> {
    match boot {
        "uki-db" if next_steps(boot, firmware).is_some() => {
            vec![
                copy::ERASE_WARNING_SECURE_BOOT,
                copy::ERASE_WARNING_UEFI_SETUP,
            ]
        }
        _ => Vec::new(),
    }
}

pub(crate) fn panel_lines(
    firmware: &crate::firmware::Firmware,
    payload: &Payload,
) -> Vec<common::ui::HeaderLine> {
    use crate::firmware::PlatformKey;
    use common::ui::HeaderLine::{Good, Pair, Row, Section, Warn};

    let mut lines = vec![
        Section(copy::PANEL_IMAGE.to_string()),
        Pair("name".to_string(), image_name(&payload.image)),
        Pair("repo".to_string(), payload.image.clone()),
        Pair("bootloader".to_string(), bootloader_said(payload)),
        Section(copy::PANEL_FIRMWARE.to_string()),
    ];
    if !firmware.efivars {
        lines.push(Row(copy::FIRMWARE_NO_EFIVARS.to_string()));
    } else {
        lines.push(match firmware.secure_boot {
            Some(true) => Good(
                copy::FIRMWARE_SECURE.to_string(),
                copy::FIRMWARE_ON.to_string(),
            ),
            Some(false) => match firmware.setup_mode {
                Some(true) => Warn(
                    copy::FIRMWARE_SECURE.to_string(),
                    copy::FIRMWARE_SETUP.to_string(),
                ),
                _ => Warn(
                    copy::FIRMWARE_SECURE.to_string(),
                    copy::FIRMWARE_OFF.to_string(),
                ),
            },
            None => Pair(
                copy::FIRMWARE_SECURE.to_string(),
                copy::FIRMWARE_UNREADABLE.to_string(),
            ),
        });
        lines.push(Pair(
            copy::PLATFORM_KEY.to_string(),
            match firmware.platform_key {
                PlatformKey::Owner => copy::PLATFORM_KEY_OWNER,
                PlatformKey::Other => copy::PLATFORM_KEY_PRESENT,
                PlatformKey::Unknown => copy::PLATFORM_KEY_UNKNOWN,
                PlatformKey::None => copy::PLATFORM_KEY_SETUP,
            }
            .to_string(),
        ));
    }
    if payload.boot.is_empty() {
        return lines;
    }
    lines.push(Section(copy::PANEL_REQUIRED.to_string()));
    if !firmware.efivars {
        lines.push(Row(copy::UKI_NEEDS_UEFI.to_string()));
        return lines;
    }
    let mut rows: Vec<&'static str> = Vec::new();
    match (
        payload.boot.as_str(),
        firmware.platform_key,
        firmware.database,
    ) {
        ("uki-db", PlatformKey::Owner, true) => {
            lines.push(Row(copy::UKI_DB_OWNER.to_string()));
            return lines;
        }
        ("uki-db", PlatformKey::None, _) => {
            lines.push(Row(copy::UKI_DB_ENROLL.to_string()));
            return lines;
        }
        ("uki-db", _, _) => rows.extend(copy::required_uki_db()),
        ("uki-shim", _, _) => rows.push(copy::shim_steps()),
        _ => {}
    }
    lines.extend(rows.into_iter().map(|line| Row(line.to_string())));
    lines
}

pub(crate) fn image_name(reference: &str) -> String {
    let last = reference.rsplit('/').next().unwrap_or(reference);
    let name = last.split([':', '@']).next().unwrap_or(last);
    match name.is_empty() {
        true => reference.to_string(),
        false => name.to_string(),
    }
}

fn bootloader_said(payload: &Payload) -> String {
    match (payload.boot.is_empty(), payload.bootloader.as_str()) {
        (false, "systemd") => "systemd-boot with a signed UKI".to_string(),
        (false, other) => format!("{other} with a signed UKI"),
        (true, "" | "grub2") => "grub2".to_string(),
        (true, other) => other.to_string(),
    }
}

fn owner_certificate(image: &str) -> Option<Vec<u8>> {
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
            "openssl",
            "x509",
            "-in",
            "/usr/share/secureboot/sb_cert.pem",
            "-outform",
            "DER",
        ])
        .output()
        .ok()?;
    match out.status.success() && !out.stdout.is_empty() {
        true => Some(out.stdout),
        false => None,
    }
}
