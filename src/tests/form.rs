use super::*;

#[test]
fn only_the_forms_named_for_a_passphrase_are_asked_for_one() {
    let wants: Vec<&str> = KINDS
        .iter()
        .map(|(name, _, _)| *name)
        .filter(|name| Encryption::wants_passphrase(name))
        .collect();
    assert_eq!(wants, ["luks-passphrase", "tpm2-luks-passphrase"]);
}

#[test]
fn the_luks_window_refuses_a_passphrase_nobody_confirmed() {
    assert_eq!(luks_short_of(NONE, "", "", "", "", true), None);
    assert_eq!(
        luks_short_of("luks-passphrase", "", "", "", "", true),
        Some(copy::still_needs(&[copy::ROW_PASSPHRASE]))
    );
    assert_eq!(
        luks_short_of("luks-passphrase", "opensesame", "", "", "", true),
        Some(copy::still_needs(&[copy::CONFIRM_PASSPHRASE]))
    );
    assert_eq!(
        luks_short_of("luks-passphrase", "opensesame", "opensesam", "", "", true),
        Some(String::new())
    );
    assert_eq!(
        luks_short_of("luks-passphrase", "opensesame", "opensesame", "", "", true),
        None
    );
    assert_eq!(
        luks_short_of("luks-passphrase", "opensesame", "opensesame", "", "", false),
        Some(copy::NO_LUKS_INITRAMFS.to_string())
    );
}

#[test]
fn the_luks_window_refuses_a_pin_nobody_confirmed() {
    assert_eq!(
        luks_short_of("tpm2-luks-pin", "", "", "", "", true),
        Some(copy::still_needs(&[copy::ROW_PIN]))
    );
    assert_eq!(
        luks_short_of("tpm2-luks-pin", "", "", "4321", "", true),
        Some(copy::still_needs(&[copy::CONFIRM_PIN]))
    );
    assert_eq!(
        luks_short_of("tpm2-luks-pin", "", "", "4321", "4320", true),
        Some(String::new())
    );
    assert_eq!(
        luks_short_of("tpm2-luks-pin", "", "", "4321", "4321", true),
        None
    );
    assert_eq!(
        luks_short_of("tpm2-luks-pin", "", "", "4321", "4321", false),
        Some(copy::NO_LUKS_INITRAMFS.to_string())
    );
    assert_eq!(
        luks_short_of("tpm2-luks", "opensesame", "", "", "", true),
        None
    );
}

#[test]
fn the_whole_disk_form_refuses_a_seeded_tpm2_kind_without_a_tpm() {
    use common::ui::{Choice, Field};
    let fields = vec![
        Field::text(copy::ROW_HOSTNAME, "deb2"),
        Field::text(copy::ROW_ACCOUNT, "buildc"),
        Field::secret(copy::ROW_PASSWORD, "hunter2"),
        Field::secret(copy::ROW_CONFIRM, "hunter2"),
        Field::pick(
            copy::ROW_LAYOUT,
            vec![
                Choice::new(copy::LAYOUT_WHOLE, ""),
                Choice::new(copy::LAYOUT_MANUAL, ""),
            ],
            Some(0),
        ),
        Field::pick(
            copy::ROW_ENCRYPTION,
            vec![Choice::new(shown("tpm2-luks"), "")],
            Some(0),
        ),
        Field::table(
            "disk",
            "disk",
            &[],
            &[],
            vec![],
            vec![],
            vec![],
            0,
            false,
            false,
        ),
        Field::secret(copy::ROW_PASSPHRASE, ""),
    ];
    assert_eq!(
        short_of(&fields, "/dev/vda", None, &a_payload(), false, None).as_deref(),
        Some(copy::NO_TPM)
    );
    assert_ne!(
        short_of(&fields, "/dev/vda", None, &a_payload(), true, None).as_deref(),
        Some(copy::NO_TPM)
    );
}

#[test]
fn the_whole_disk_form_refuses_a_seeded_pin_kind_with_no_pin() {
    use common::ui::{Choice, Field};
    let fields = vec![
        Field::text(copy::ROW_HOSTNAME, "deb2"),
        Field::text(copy::ROW_ACCOUNT, "buildc"),
        Field::secret(copy::ROW_PASSWORD, "hunter2"),
        Field::secret(copy::ROW_CONFIRM, "hunter2"),
        Field::pick(
            copy::ROW_LAYOUT,
            vec![
                Choice::new(copy::LAYOUT_WHOLE, ""),
                Choice::new(copy::LAYOUT_MANUAL, ""),
            ],
            Some(0),
        ),
        Field::pick(
            copy::ROW_ENCRYPTION,
            vec![Choice::new(shown("tpm2-luks-pin"), "")],
            Some(0),
        ),
        Field::table(
            "disk",
            "disk",
            &[],
            &[],
            vec![],
            vec![],
            vec![],
            0,
            false,
            false,
        ),
        Field::secret(copy::ROW_PASSPHRASE, ""),
        Field::secret(copy::ROW_PIN, ""),
    ];
    assert_eq!(
        short_of(&fields, "/dev/vda", None, &a_payload(), true, None).as_deref(),
        Some(copy::still_needs(&[copy::ROW_PIN]).as_str())
    );
}

#[test]
fn the_confirm_screen_shows_what_is_about_to_be_erased() {
    let answers = Answers {
        disk: "/dev/vda".to_string(),
        hostname: "deb2".to_string(),
        user: "buildc".to_string(),
        password: "hunter2".to_string(),
        opened: Opened::Keep,
        encryption: Encryption {
            kind: NONE.to_string(),
            passphrase: String::new(),
            pin: String::new(),
        },
        layout: None,
    };
    assert_eq!(
        answers.summary(&a_payload()),
        vec![
            (copy::ROW_DISK.to_string(), "/dev/vda".to_string()),
            (copy::ROW_HOSTNAME.to_string(), "deb2".to_string()),
            (copy::ROW_ACCOUNT.to_string(), "buildc".to_string()),
            (
                copy::ROW_PASSWORD.to_string(),
                copy::PASSWORD_SET.to_string()
            ),
            (copy::ROW_ENCRYPTION.to_string(), copy::ENC_NONE.to_string()),
            ("esp".to_string(), "2.0 GB  fat32".to_string()),
            ("/boot".to_string(), "2.0 GB  ext4".to_string()),
            ("root".to_string(), "the remaining disk  ext4".to_string()),
        ]
    );
    let question = copy::erasing(&answers.disk);
    assert!(
        question.contains("Erase everything on /dev/vda"),
        "{question}"
    );
    assert!(!question.contains("hunter2"));
}

#[test]
fn the_passphrase_is_never_a_row_and_the_manual_rows_come_and_go() {
    use common::ui::{Choice, Field};
    let form = |manual: bool, encryption: Field| {
        vec![
            Field::text(copy::ROW_HOSTNAME, "deb2"),
            Field::text(copy::ROW_ACCOUNT, "buildc"),
            Field::secret(copy::ROW_PASSWORD, "hunter2"),
            Field::secret(copy::ROW_CONFIRM, "hunter2"),
            Field::pick(
                copy::ROW_LAYOUT,
                vec![
                    Choice::new(copy::LAYOUT_WHOLE, ""),
                    Choice::new(copy::LAYOUT_MANUAL, ""),
                ],
                Some(usize::from(manual)),
            ),
            encryption,
            Field::table(
                "disk",
                "disk",
                &[],
                &[],
                vec![],
                vec![],
                vec![],
                0,
                false,
                false,
            ),
            Field::secret(copy::ROW_PASSPHRASE, ""),
        ]
    };
    let kinds = |kind: &str| Field::action(copy::ROW_ENCRYPTION, shown(kind));
    for kind in [NONE, "tpm2-luks", "luks-passphrase", "tpm2-luks-passphrase"] {
        assert!(!asked(&form(false, kinds(kind))).contains(&ROW_PASSPHRASE));
    }
    assert_eq!(asked(&form(false, kinds(NONE))).len(), 7);
    let bare = asked(&form(true, kinds(NONE)));
    assert!(!bare.contains(&ROW_ENCRYPTION));
    assert!(bare.contains(&ROW_TABLE));
    let opened = Field::pick(
        copy::ROW_ENCRYPTION,
        vec![Choice::new(copy::OPENED_KEEP, "")],
        Some(0),
    );
    assert!(asked(&form(true, opened)).contains(&ROW_ENCRYPTION));
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "every case reads the form fixtures the closures at the top build"
)]
fn the_action_says_what_the_form_is_short_of() {
    use common::ui::{Choice, Field};
    let form = |password: &str, confirm: &str, kind: &str, passphrase: &str| {
        vec![
            Field::text(copy::ROW_HOSTNAME, "deb2"),
            Field::text(copy::ROW_ACCOUNT, "buildc"),
            Field::secret(copy::ROW_PASSWORD, password),
            Field::secret(copy::ROW_CONFIRM, confirm),
            Field::pick(
                copy::ROW_LAYOUT,
                vec![
                    Choice::new(copy::LAYOUT_WHOLE, ""),
                    Choice::new(copy::LAYOUT_MANUAL, ""),
                ],
                Some(0),
            ),
            Field::pick(
                copy::ROW_ENCRYPTION,
                vec![Choice::new(shown(kind), "")],
                Some(0),
            ),
            Field::table(
                "disk",
                "disk",
                &[],
                &[],
                vec![],
                vec![],
                vec![],
                0,
                false,
                false,
            ),
            Field::secret(copy::ROW_PASSPHRASE, passphrase),
        ]
    };
    let manual = |mut fields: Vec<common::ui::Field>| {
        fields[ROW_LAYOUT] = Field::pick(
            copy::ROW_LAYOUT,
            vec![
                Choice::new(copy::LAYOUT_WHOLE, ""),
                Choice::new(copy::LAYOUT_MANUAL, ""),
            ],
            Some(1),
        );
        fields
    };
    assert!(short_of(
        &form("hunter2", "hunter2", NONE, ""),
        "/dev/vda",
        None,
        &a_payload(),
        true,
        None
    )
    .is_none());
    let differ = short_of(
        &form("hunter2", "hunter3", NONE, ""),
        "/dev/vda",
        None,
        &a_payload(),
        true,
        None,
    )
    .unwrap();
    assert_eq!(differ, "");
    assert!(short_of(
        &form("hunter2", "hunter2", "luks-passphrase", ""),
        "/dev/vda",
        None,
        &a_payload(),
        true,
        None
    )
    .is_some());
    assert!(short_of(
        &form("hunter2", "hunter2", "luks-passphrase", "x"),
        "/dev/vda",
        None,
        &a_payload(),
        true,
        None
    )
    .is_none());
    assert_eq!(
        short_of(
            &form("hunter2", "hunter2", "luks-passphrase", "x"),
            "/dev/vda",
            None,
            &Payload {
                luks_initramfs: false,
                ..a_payload()
            },
            true,
            None
        )
        .as_deref(),
        Some(copy::NO_LUKS_INITRAMFS)
    );
    let mut bare = form("", "", NONE, "");
    bare[ROW_ACCOUNT] = Field::text(copy::ROW_ACCOUNT, "");
    let short = short_of(&bare, "/dev/vda", None, &a_payload(), true, None).unwrap();
    assert!(
        short.contains(copy::ROW_ACCOUNT) && short.contains(copy::ROW_PASSWORD),
        "{short}"
    );

    let root_keyfile = CustomLayout {
        disk: "/dev/vda".to_string(),
        mounts: vec![CustomMount {
            partition: "/dev/vda1".to_string(),
            target: "/boot/efi".to_string(),
            fstype: "unformatted".to_string(),

            passphrase: String::new(),
        }],
        opens: vec![LuksOpen {
            partition: "/dev/vda3".to_string(),
            target: "/".to_string(),
            key: Key::Data(b"key".to_vec()),
        }],
        ..Default::default()
    };
    let opens_row = |label: &str| {
        let mut fields = manual(form("hunter2", "hunter2", NONE, ""));
        fields[ROW_ENCRYPTION] =
            Field::pick(copy::ROW_ENCRYPTION, vec![Choice::new(label, "")], Some(0));
        fields
    };
    assert_eq!(
        short_of(
            &opens_row(copy::OPENED_KEEP),
            "/dev/vda",
            Some(&root_keyfile),
            &a_payload(),
            true,
            None
        )
        .as_deref(),
        Some(copy::OPENED_ROOT_KEYFILE)
    );
    assert_eq!(
        short_of(
            &opens_row(copy::OPENED_TPM2),
            "/dev/vda",
            Some(&root_keyfile),
            &a_payload(),
            true,
            None
        )
        .as_deref(),
        Some(copy::OPENED_ROOT_KEYFILE)
    );

    let plain_root = CustomLayout {
        disk: "/dev/vda".to_string(),
        mounts: vec![
            CustomMount {
                partition: "/dev/vda1".to_string(),
                target: "/boot/efi".to_string(),
                fstype: "unformatted".to_string(),

                passphrase: String::new(),
            },
            CustomMount {
                partition: "/dev/vda2".to_string(),
                target: "/".to_string(),
                fstype: "ext4".to_string(),

                passphrase: String::new(),
            },
        ],
        opens: vec![LuksOpen {
            partition: "/dev/vda3".to_string(),
            target: "/var".to_string(),
            key: Key::Passphrase("opensesame".to_string()),
        }],
        ..Default::default()
    };
    assert!(short_of(
        &opens_row(copy::OPENED_KEEP),
        "/dev/vda",
        Some(&plain_root),
        &a_payload(),
        true,
        None
    )
    .is_none());
    for answer in [copy::OPENED_ADD_KEY, copy::OPENED_TPM2] {
        assert_eq!(
            short_of(
                &opens_row(answer),
                "/dev/vda",
                Some(&plain_root),
                &a_payload(),
                true,
                None
            )
            .as_deref(),
            Some(copy::OPENED_KEYFILE_PLAIN),
            "{answer}"
        );
    }
    let stale = CustomLayout {
        opens: vec![LuksOpen {
            partition: "/dev/vda3".to_string(),
            target: "/var".to_string(),
            key: Key::File("/keys/var.key".into()),
        }],
        ..plain_root.clone()
    };
    assert_eq!(
        short_of(
            &opens_row(copy::OPENED_KEEP),
            "/dev/vda",
            Some(&stale),
            &a_payload(),
            true,
            None
        )
        .as_deref(),
        Some(copy::OPENED_KEYFILE_PLAIN)
    );
}
