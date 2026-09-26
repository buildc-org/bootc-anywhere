use super::*;

#[test]
fn a_created_partition_answers_the_same_rules() {
    let create = |gb: u64, target: &str, fstype: &str| Created {
        gb,
        target: target.to_string(),
        fstype: fstype.to_string(),
        ..Default::default()
    };
    let held = |creates: Vec<Created>| CustomLayout {
        disk: "/dev/vda".to_string(),
        creates,
        ..Default::default()
    };
    assert_eq!(
        layout_short_of(&held(vec![create(20, "/", "")]), false, None, 0).as_deref(),
        Some(copy::CUSTOM_UNFORMATTED)
    );
    assert_eq!(
        layout_short_of(&held(vec![create(20, "", "ext4")]), false, None, 0).as_deref(),
        Some(copy::CUSTOM_UNMOUNTED)
    );
    let whole = held(vec![
        create(2, "/boot/efi", "fat32"),
        create(60, "/", "btrfs"),
    ]);
    assert!(layout_short_of(&whole, false, None, 0).is_none());
    assert!(layout_short_of(&whole, true, None, 0).is_none());
    let xfs = held(vec![
        create(2, "/boot/efi", "fat32"),
        create(60, "/", "xfs"),
    ]);
    assert!(layout_short_of(&xfs, false, None, 0).is_none());
    assert_eq!(
        layout_short_of(&xfs, true, None, 0).as_deref(),
        Some(copy::CUSTOM_VERITY)
    );
    let mut clash = whole.clone();
    clash.mounts.push(CustomMount {
        partition: "/dev/vda9".to_string(),
        target: "/".to_string(),
        fstype: "ext4".to_string(),
        passphrase: String::new(),
    });
    assert_eq!(
        layout_short_of(&clash, false, None, 0).as_deref(),
        Some(copy::CUSTOM_DUPLICATE)
    );
    assert_eq!(
        layout_short_of(&held(vec![create(60, "/", "btrfs")]), false, None, 0).as_deref(),
        Some(copy::CUSTOM_ESP)
    );
}

#[test]
fn the_confirmation_names_every_partition_it_will_remove() {
    let payload = a_payload();
    let answers = Answers {
        disk: "/dev/sda".to_string(),
        hostname: "deb2".to_string(),
        user: "buildc".to_string(),
        password: "hunter2".to_string(),
        opened: Opened::Keep,
        encryption: Encryption {
            kind: NONE.to_string(),
            passphrase: String::new(),
            pin: String::new(),
        },
        layout: Some(CustomLayout {
            disk: "/dev/sda".to_string(),
            deletes: vec![
                "/dev/sda1".to_string(),
                "/dev/sda2".to_string(),
                "/dev/sda3".to_string(),
            ],
            creates: vec![
                Created {
                    gb: 2,
                    target: "/boot/efi".to_string(),
                    fstype: "fat32".to_string(),
                    device: String::new(),
                    ..Default::default()
                },
                Created {
                    gb: 400,
                    target: "/".to_string(),
                    fstype: "btrfs".to_string(),
                    device: String::new(),
                    ..Default::default()
                },
            ],
            esp: vec![EspRemoval {
                partition: "/dev/sda1".to_string(),
                entries: vec!["fedora".to_string()],
            }],
            ..Default::default()
        }),
    };
    let said = answers.summary(&payload);
    let flat: String = said
        .iter()
        .map(|(label, value)| format!("{label} {value}\n"))
        .collect();
    for device in ["/dev/sda1", "/dev/sda2", "/dev/sda3"] {
        assert!(
            flat.contains(device),
            "{device} is not on the screen:\n{flat}"
        );
    }
    assert_eq!(flat.matches("contents are lost").count(), 3, "{flat}");
    assert!(
        flat.contains(&copy::summary_esp("fedora", "/dev/sda1")),
        "{flat}"
    );
    assert!(flat.contains("400 GB"), "{flat}");
    assert!(flat.contains("/boot/efi"), "{flat}");
    let asked = copy::removing_partitions("/dev/sda", 3);
    assert!(asked.contains('3') && asked.contains("Delete"), "{asked}");
    assert!(copy::removing_partitions("/dev/sda", 1).starts_with("Delete 1 partition "));
}

#[test]
fn the_confirmation_names_the_format_inside_an_opened_root() {
    let payload = a_payload();
    let answers = Answers {
        disk: "/dev/sda".to_string(),
        hostname: "deb2".to_string(),
        user: "buildc".to_string(),
        password: "hunter2".to_string(),
        opened: Opened::Keep,
        encryption: Encryption {
            kind: NONE.to_string(),
            passphrase: String::new(),
            pin: String::new(),
        },
        layout: Some(CustomLayout {
            disk: "/dev/sda".to_string(),
            opens: vec![LuksOpen {
                partition: "/dev/sda3".to_string(),
                target: "/".to_string(),
                key: Key::Passphrase("opensesame".to_string()),
            }],
            ..Default::default()
        }),
    };
    let said = answers.summary(&payload);
    assert!(
        said.contains(&(
            "/".to_string(),
            copy::opened_root("/dev/sda3", &payload.filesystem, copy::BOOT_PASSPHRASE)
        )),
        "{said:?}"
    );
}

#[test]
fn custom_layouts_need_one_root_and_unique_mount_points() {
    let layout = |targets: &[&str]| CustomLayout {
        disk: "/dev/vda".to_string(),
        mounts: targets
            .iter()
            .enumerate()
            .map(|(at, target)| CustomMount {
                partition: format!("/dev/vda{}", at + 1),
                passphrase: String::new(),
                target: target.to_string(),
                fstype: "ext4".to_string(),
            })
            .collect(),
        opens: Vec::new(),
        ..Default::default()
    };
    assert_eq!(
        layout_short_of(&layout(&["/var"]), false, None, 0).as_deref(),
        Some(copy::CUSTOM_ROOT)
    );
    assert_eq!(
        layout_short_of(&layout(&["/", "/"]), false, None, 0).as_deref(),
        Some(copy::CUSTOM_DUPLICATE)
    );
    assert!(layout_short_of(&layout(&["/boot/efi", "/", "/var"]), false, None, 0).is_none());
    let mut twice = layout(&["/boot/efi", "/var"]);
    twice.opens.push(LuksOpen {
        partition: "/dev/vda9".to_string(),
        target: "/".to_string(),
        key: Key::Passphrase("x".to_string()),
    });
    assert!(layout_short_of(&twice, false, None, 0).is_none());
    twice.mounts.push(CustomMount {
        partition: "/dev/vda8".to_string(),
        target: "/".to_string(),
        fstype: "ext4".to_string(),

        passphrase: String::new(),
    });
    assert_eq!(
        layout_short_of(&twice, false, None, 0).as_deref(),
        Some(copy::CUSTOM_DUPLICATE)
    );
}

#[test]
fn a_passphrase_never_reads_back_out_of_a_key() {
    let said = format!("{:?}", Key::Passphrase("opensesame".to_string()));
    assert!(!said.contains("opensesame"), "{said}");
    let said = format!("{:?}", Key::Data(b"opensesame".to_vec()));
    assert!(!said.contains("opensesame"), "{said}");
}

#[test]
fn the_ladder_says_how_each_container_opens_at_boot() {
    let data = |key: Key| LuksOpen {
        partition: "/dev/vda3".to_string(),
        target: "/var".to_string(),
        key,
    };
    let root = LuksOpen {
        partition: "/dev/vda3".to_string(),
        target: "/".to_string(),
        key: Key::Data(b"opensesame".to_vec()),
    };
    assert_eq!(at_boot(&root, Opened::Keep), copy::OPENED_ROOT_KEYFILE);
    assert_eq!(at_boot(&root, Opened::Tpm2), copy::BOOT_TPM2);
    let root_passphrase = LuksOpen {
        partition: "/dev/vda3".to_string(),
        target: "/".to_string(),
        key: Key::Passphrase("opensesame".to_string()),
    };
    assert_eq!(
        at_boot(&root_passphrase, Opened::Keep),
        copy::BOOT_PASSPHRASE
    );
    assert_eq!(
        at_boot(&data(Key::Data(b"key".to_vec())), Opened::Keep),
        copy::BOOT_KEYFILE
    );
    assert_eq!(
        at_boot(&data(Key::Passphrase("x".into())), Opened::Keep),
        copy::BOOT_PASSPHRASE
    );
    assert_eq!(
        at_boot(&data(Key::Passphrase("x".into())), Opened::AddKey),
        copy::BOOT_ADDED_KEY
    );
    assert_eq!(
        at_boot(&data(Key::Passphrase("x".into())), Opened::Tpm2),
        copy::BOOT_TPM2
    );
}

#[test]
fn the_opened_rows_answer_reads_back() {
    assert_eq!(Opened::of(copy::OPENED_TPM2), Opened::Tpm2);
    assert_eq!(Opened::of(copy::OPENED_ADD_KEY), Opened::AddKey);
    assert_eq!(Opened::of(copy::OPENED_KEEP), Opened::Keep);
    assert_eq!(Opened::of("anything else"), Opened::Keep);
}

#[test]
fn a_headless_run_refuses_a_disk_it_cannot_read() {
    let given = Given {
        disk: Some("/nonexistent/buildc-none".to_string()),
        hostname: Some("deb2".to_string()),
        user: Some("buildc".to_string()),
        password: Some("hunter2".to_string()),
        ..Default::default()
    };
    match Answers::seeded(&a_payload(), given, &Prompt::silent()) {
        Err(said) => assert!(said.contains("lsblk"), "{said}"),
        Ok(_) => panic!("a disk lsblk cannot read is refused"),
    }
}
