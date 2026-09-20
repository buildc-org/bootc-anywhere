use super::*;

#[test]
fn every_installer_screen_string_fits_the_narrowest_console() {
    let rows = [
        ("INSTALL_DONE", copy::INSTALL_DONE),
        ("DONE_KEYS", copy::DONE_KEYS),
        ("RECOVERY_HEADING", copy::RECOVERY_HEADING),
        ("NEXT_STEPS_HEADING", copy::NEXT_STEPS_HEADING),
        ("write_down", copy::write_down()),
        ("RESTART", copy::RESTART),
        ("SHUT_DOWN", copy::SHUT_DOWN),
        ("INSTALL", copy::INSTALL),
        ("GO_BACK", copy::GO_BACK),
        ("INSTALLATION_SUMMARY", copy::INSTALLATION_SUMMARY),
        ("READY", copy::READY),
        ("START_INSTALLATION", copy::START_INSTALLATION),
        ("ERASE_WARNING_SECURE_BOOT", copy::ERASE_WARNING_SECURE_BOOT),
        ("ERASE_WARNING_UEFI_SETUP", copy::ERASE_WARNING_UEFI_SETUP),
        ("OPENED_ADD_KEY", copy::OPENED_ADD_KEY),
        ("OPENED_KEYFILE_PLAIN", copy::OPENED_KEYFILE_PLAIN),
        ("PASSWORD_SET", copy::PASSWORD_SET),
        ("ROW_ENCRYPTION", copy::ROW_ENCRYPTION),
    ];
    for (name, text) in rows {
        let wide = text.chars().count();
        assert!(
            wide <= common::ui::ROW_ROOM,
            "{name} is {wide} columns and a list row has {}",
            common::ui::ROW_ROOM
        );
    }
}

#[test]
fn the_panel_states_the_firmware_and_what_the_image_needs() {
    use crate::firmware::{Firmware, PlatformKey};
    use common::ui::HeaderLine;
    let payload = |boot: &str| Payload {
        recipe: "/mnt/tect/install-recipe.json".into(),
        image: "ghcr.io/buildc-org/deb2:latest".to_string(),
        hostname: "deb2".to_string(),
        filesystem: "ext4".to_string(),
        bootloader: "grub2".to_string(),
        boot: boot.to_string(),
        composefs: false,
        reserve: std::sync::OnceLock::new(),
        luks_initramfs: true,
    };
    let payload_of = |boot: &str, bootloader: &str| Payload {
        bootloader: bootloader.to_string(),
        ..payload(boot)
    };
    let state = |platform_key, database| Firmware {
        efivars: true,
        secure_boot: Some(true),
        setup_mode: Some(false),
        platform_key,
        database,
    };
    let setup = |platform_key| Firmware {
        setup_mode: Some(true),
        ..state(platform_key, false)
    };
    let text = |lines: &[HeaderLine]| -> Vec<String> {
        lines
            .iter()
            .map(|line| match line {
                HeaderLine::Section(title) | HeaderLine::Row(title) => title.clone(),
                HeaderLine::Pair(label, value)
                | HeaderLine::Good(label, value)
                | HeaderLine::Warn(label, value) => format!("{label}: {value}"),
            })
            .collect()
    };
    let said = |lines: Vec<String>, phrase: &str| lines.iter().any(|line| line == phrase);

    let lines = text(&panel_lines(&state(PlatformKey::None, false), &payload("")));
    assert!(said(lines.clone(), copy::PANEL_IMAGE), "{lines:?}");
    assert!(said(lines.clone(), "name: deb2"), "{lines:?}");
    assert!(
        said(lines.clone(), "repo: ghcr.io/buildc-org/deb2:latest"),
        "{lines:?}"
    );
    assert!(
        said(lines.clone(), "bootloader: grub2 (the base's vendor chain)"),
        "{lines:?}"
    );
    assert!(!said(lines.clone(), copy::PANEL_REQUIRED), "{lines:?}");

    let lines = text(&panel_lines(
        &state(PlatformKey::Other, false),
        &payload_of("uki-db", "systemd"),
    ));
    assert!(
        said(lines.clone(), "bootloader: systemd-boot with a signed UKI"),
        "{lines:?}"
    );
    assert!(said(lines.clone(), copy::PANEL_REQUIRED), "{lines:?}");
    assert!(said(lines.clone(), copy::required_uki_db()[0]), "{lines:?}");
    assert!(
        said(lines.clone(), copy::required_uki_db()[2]),
        "the dual-boot cost is on the panel: {lines:?}"
    );
    assert!(
        said(
            lines.clone(),
            &format!("{}: {}", copy::PLATFORM_KEY, copy::PLATFORM_KEY_PRESENT),
        ),
        "{lines:?}"
    );
    assert!(!said(lines.clone(), copy::UKI_DB_OWNER), "{lines:?}");

    let lines = text(&panel_lines(
        &setup(PlatformKey::None),
        &payload_of("uki-db", "systemd"),
    ));
    assert!(
        said(
            lines.clone(),
            &format!("{}: {}", copy::PLATFORM_KEY, copy::PLATFORM_KEY_SETUP),
        ),
        "{lines:?}"
    );
    assert!(said(lines.clone(), copy::UKI_DB_ENROLL), "{lines:?}");
    assert!(
        !said(lines.clone(), copy::required_uki_db()[0]),
        "{lines:?}"
    );

    let lines = text(&panel_lines(
        &state(PlatformKey::Owner, true),
        &payload_of("uki-db", "systemd"),
    ));
    assert!(
        said(
            lines.clone(),
            &format!("{}: {}", copy::PLATFORM_KEY, copy::PLATFORM_KEY_OWNER),
        ),
        "{lines:?}"
    );
    assert!(said(lines.clone(), copy::UKI_DB_OWNER), "{lines:?}");
    assert!(
        !said(lines.clone(), copy::required_uki_db()[0]),
        "{lines:?}"
    );

    let lines = text(&panel_lines(
        &state(PlatformKey::Owner, false),
        &payload_of("uki-db", "systemd"),
    ));
    assert!(said(lines.clone(), copy::required_uki_db()[0]), "{lines:?}");
    assert!(!said(lines.clone(), copy::UKI_DB_OWNER), "{lines:?}");

    let lines = text(&panel_lines(
        &state(PlatformKey::Unknown, false),
        &payload_of("uki-db", "systemd"),
    ));
    assert!(
        said(
            lines.clone(),
            &format!("{}: {}", copy::PLATFORM_KEY, copy::PLATFORM_KEY_UNKNOWN),
        ),
        "{lines:?}"
    );
    assert!(said(lines.clone(), copy::required_uki_db()[0]), "{lines:?}");

    let lines = text(&panel_lines(
        &state(PlatformKey::Other, false),
        &payload_of("uki-shim", "systemd"),
    ));
    assert!(said(lines.clone(), copy::shim_steps()), "{lines:?}");
    assert!(
        !said(lines.clone(), copy::required_uki_db()[0]),
        "{lines:?}"
    );

    let lines = text(&panel_lines(
        &Firmware::default(),
        &payload_of("uki-db", "systemd"),
    ));
    assert!(said(lines.clone(), copy::PANEL_FIRMWARE), "{lines:?}");
    assert!(said(lines.clone(), copy::FIRMWARE_NO_EFIVARS), "{lines:?}");
    assert!(said(lines.clone(), copy::UKI_NEEDS_UEFI), "{lines:?}");
    assert!(!said(lines.clone(), copy::UKI_DB_ENROLL), "{lines:?}");
}

#[test]
fn the_last_screen_asks_for_the_firmware_steps_the_panel_saw() {
    use crate::firmware::{Firmware, PlatformKey};
    let with = |platform_key, database| Firmware {
        efivars: true,
        secure_boot: Some(true),
        setup_mode: Some(false),
        platform_key,
        database,
    };
    assert_eq!(
        next_steps("uki-db", &with(PlatformKey::Other, false)),
        Some(copy::next_steps_setup())
    );
    assert_eq!(next_steps("uki-db", &with(PlatformKey::None, false)), None);
    assert_eq!(next_steps("uki-db", &with(PlatformKey::Owner, true)), None);
    assert_eq!(
        next_steps("uki-db", &with(PlatformKey::Owner, false)),
        Some(copy::next_steps_setup())
    );
    assert_eq!(
        next_steps("uki-shim", &with(PlatformKey::Other, false)),
        Some(copy::shim_steps())
    );
    assert_eq!(next_steps("", &with(PlatformKey::Other, false)), None);
    assert_eq!(next_steps("uki-db", &Firmware::default()), None);
}

#[test]
fn the_erase_warning_matches_the_cases_next_steps_asks_for() {
    use crate::firmware::{Firmware, PlatformKey};
    let with = |platform_key, database| Firmware {
        efivars: true,
        secure_boot: Some(true),
        setup_mode: Some(false),
        platform_key,
        database,
    };
    assert_eq!(
        erase_warning("uki-db", &with(PlatformKey::Other, false)),
        vec![
            copy::ERASE_WARNING_SECURE_BOOT,
            copy::ERASE_WARNING_UEFI_SETUP
        ]
    );
    assert_eq!(
        erase_warning("uki-db", &with(PlatformKey::Owner, false)),
        vec![
            copy::ERASE_WARNING_SECURE_BOOT,
            copy::ERASE_WARNING_UEFI_SETUP
        ]
    );
    assert_eq!(
        erase_warning("uki-db", &with(PlatformKey::Unknown, false)),
        vec![
            copy::ERASE_WARNING_SECURE_BOOT,
            copy::ERASE_WARNING_UEFI_SETUP
        ]
    );
    assert!(erase_warning("uki-db", &with(PlatformKey::None, false)).is_empty());
    assert!(erase_warning("uki-db", &with(PlatformKey::Owner, true)).is_empty());
    assert!(erase_warning("uki-shim", &with(PlatformKey::Other, false)).is_empty());
    assert!(erase_warning("", &with(PlatformKey::Other, false)).is_empty());
    assert!(erase_warning("uki-db", &Firmware::default()).is_empty());
}
