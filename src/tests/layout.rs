use super::*;

fn cut_whole_disk(name: &str, payload: &Payload, kind: &str) -> (CustomLayout, Vec<String>) {
    require_sfdisk();
    let root = scratch(name);
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(32 * 1024 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let mut child = Command::new("sfdisk")
        .args(["-q", &disk])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("sfdisk");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"label: dos\nsize=20M\nsize=20M\n")
        .expect("the starting table");
    assert!(child.wait().expect("sfdisk").success());
    let encryption = Encryption {
        kind: kind.to_string(),
        passphrase: String::new(),
        pin: String::new(),
    };
    let layout = whole_disk(&disk, payload, &encryption).expect("a whole-disk plan");
    super::apply_cuts(&layout).expect("the cut");
    let dump = Command::new("sfdisk")
        .args(["--dump", &disk])
        .output()
        .expect("a dump");
    let lines = String::from_utf8_lossy(&dump.stdout)
        .lines()
        .filter(|line| line.starts_with(&disk))
        .map(|line| line.to_uppercase())
        .collect();
    let _ = std::fs::remove_dir_all(&root);
    (layout, lines)
}

fn a_payload_for(bootloader: &str, composefs: bool) -> Payload {
    let mut payload = a_payload();
    payload.bootloader = bootloader.to_string();
    payload.composefs = composefs;
    payload.filesystem = "btrfs".to_string();
    payload
}

const ESP_TYPE: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
const LINUX_TYPE: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";

#[test]
fn a_grub_disk_is_cut_into_an_esp_a_boot_and_a_root() {
    for composefs in [false, true] {
        let payload = a_payload_for("grub2", composefs);
        let (layout, lines) = cut_whole_disk("whole-grub", &payload, NONE);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].contains(ESP_TYPE) && lines[0].contains("EFI-SYSTEM"));
        assert!(lines[1].contains(LINUX_TYPE) && lines[1].contains("\"BOOT\""));
        assert!(lines[2].contains(LINUX_TYPE) && lines[2].contains("\"ROOT\""));
        let targets: Vec<(&str, &str)> = layout
            .creates
            .iter()
            .map(|create| (create.target.as_str(), create.fstype.as_str()))
            .collect();
        assert_eq!(
            targets,
            [("/boot/efi", "fat32"), ("/boot", "ext4"), ("/", "btrfs")]
        );
    }
}

#[test]
fn a_systemd_boot_disk_is_cut_into_an_esp_and_a_root() {
    for (composefs, root_type) in [(false, LINUX_TYPE), (true, ROOT_GUID)] {
        let payload = a_payload_for("systemd", composefs);
        let (_, lines) = cut_whole_disk("whole-sdboot", &payload, NONE);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains(ESP_TYPE));
        assert!(
            lines[1].contains(&root_type.to_uppercase()),
            "composefs {composefs}: {}",
            lines[1]
        );
    }
}

#[test]
fn the_whole_disk_root_takes_the_rest_of_the_disk() {
    let payload = a_payload_for("grub2", false);
    let (layout, _) = cut_whole_disk("whole-rest", &payload, NONE);
    let root = layout.creates.last().expect("a root");
    assert_eq!(root.gb, 30);
}

#[test]
fn an_encrypted_answer_seals_the_root_alone() {
    for kind in ["luks-passphrase", "tpm2-luks", "tpm2-luks-pin"] {
        let payload = a_payload_for("grub2", false);
        let (layout, _) = cut_whole_disk("whole-sealed", &payload, kind);
        let sealed: Vec<&str> = layout
            .creates
            .iter()
            .filter(|create| create.encrypt)
            .map(|create| create.target.as_str())
            .collect();
        assert_eq!(sealed, ["/"], "{kind}");
    }
    let payload = a_payload_for("grub2", false);
    let (layout, _) = cut_whole_disk("whole-plain", &payload, NONE);
    assert!(layout.creates.iter().all(|create| !create.encrypt));
}

#[test]
fn a_plan_the_payload_cannot_answer_refuses_before_the_cut() {
    let root = scratch("whole-refused");
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(32 * 1024 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let none = Encryption {
        kind: NONE.to_string(),
        passphrase: String::new(),
        pin: String::new(),
    };
    let mut unbooted = a_payload_for("grub2", false);
    unbooted.bootloader = String::new();
    assert!(whole_disk(&disk, &unbooted, &none).is_err());
    let mut unformatted = a_payload_for("grub2", false);
    unformatted.filesystem = String::new();
    assert!(whole_disk(&disk, &unformatted, &none).is_err());
    std::fs::File::options()
        .write(true)
        .open(&image)
        .and_then(|file| file.set_len(12 * 1024 * 1024 * 1024))
        .expect("a smaller disk");
    let small = whole_disk(&disk, &a_payload_for("grub2", false), &none)
        .expect_err("a root below the reserve");
    assert!(small.contains("below"), "{small}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn only_an_ext4_root_is_made_with_verity() {
    assert_eq!(
        mkfs_args("ext4", "/").expect("ext4"),
        ["mkfs.ext4", "-F", "-O", "verity"]
    );
    assert_eq!(
        mkfs_args("ext4", "/boot").expect("ext4"),
        ["mkfs.ext4", "-F"]
    );
    assert_eq!(
        mkfs_args("fat32", "/boot/efi").expect("fat32"),
        ["mkfs.fat", "-F", "32"]
    );
}

#[test]
fn a_filesystem_the_installer_cannot_write_is_refused() {
    for fstype in ["zfs", "", "luks", "ext3"] {
        assert!(mkfs_args(fstype, "/").is_err(), "{fstype:?}");
    }
}

#[test]
fn a_sealed_root_is_formatted_through_its_mapper() {
    let layout = CustomLayout {
        disk: "/dev/vda".to_string(),
        creates: vec![
            Created {
                gb: 2,
                target: "/boot/efi".to_string(),
                fstype: "fat32".to_string(),
                device: "/dev/vda1".to_string(),
                ..Default::default()
            },
            Created {
                gb: 30,
                target: "/".to_string(),
                fstype: "btrfs".to_string(),
                device: "/dev/vda2".to_string(),
                encrypt: true,
                ..Default::default()
            },
        ],
        opens: vec![LuksOpen {
            partition: "/dev/vda2".to_string(),
            target: "/".to_string(),
            key: Key::Passphrase("opensesame".to_string()),
        }],
        ..Default::default()
    };
    assert_eq!(
        volumes(&layout, "xfs"),
        [
            Volume {
                device: "/dev/vda1".to_string(),
                target: "/boot/efi".to_string(),
                format: Some("fat32".to_string()),
            },
            Volume {
                device: "/dev/mapper/buildc-1".to_string(),
                target: "/".to_string(),
                format: Some("btrfs".to_string()),
            },
        ]
    );
}

#[test]
fn an_opened_root_is_formatted_inside_and_a_kept_partition_is_not() {
    let layout = CustomLayout {
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
            key: Key::Passphrase("opensesame".to_string()),
        }],
        ..Default::default()
    };
    assert_eq!(
        volumes(&layout, "ext4")
            .iter()
            .map(|volume| (volume.target.as_str(), volume.format.as_deref()))
            .collect::<Vec<_>>(),
        [("/boot/efi", None), ("/", Some("ext4"))]
    );
}

#[test]
fn a_mount_point_mounts_after_the_one_it_sits_under() {
    let volume = |target: &str| Volume {
        device: format!("/dev/{}", target.len()),
        target: target.to_string(),
        format: None,
    };
    let volumes = [
        volume("/boot/efi"),
        volume("/swap"),
        volume("/"),
        volume("/boot"),
    ];
    let order: Vec<&str> = mount_order(&volumes)
        .into_iter()
        .map(|volume| volume.target.as_str())
        .collect();
    assert_eq!(order, ["/", "/boot", "/boot/efi"]);
}

#[test]
fn a_program_missing_from_path_is_not_found() {
    assert!(on_path("sh"));
    assert!(!on_path("bootc-anywhere-no-such-mkfs"));
}

#[test]
fn only_the_target_disks_buildc_mappers_are_stale() {
    let root = scratch("stale-mappers");
    let blocks = root.join("block");
    let devices = root.join("devices");
    let mapper = |dm: &str, name: &str, disk: &str, part: &str| {
        let at = blocks.join(dm);
        std::fs::create_dir_all(at.join("dm")).expect("a dm directory");
        std::fs::create_dir_all(at.join("slaves")).expect("a slaves directory");
        std::fs::write(at.join("dm/name"), format!("{name}\n")).expect("a dm name");
        let partition = devices.join(disk).join(part);
        std::fs::create_dir_all(&partition).expect("a partition directory");
        std::os::unix::fs::symlink(&partition, at.join("slaves").join(part)).expect("a slave link");
    };
    mapper("dm-0", "buildc-1", "vda", "vda3");
    mapper("dm-1", "buildc-1x", "vdb", "vdb2");
    mapper("dm-2", "luks-old", "vda", "vda4");
    std::fs::create_dir_all(blocks.join("vda")).expect("a plain disk entry");
    assert_eq!(
        stale_mappers(&blocks, "/dev/vda").expect("a sysfs listing"),
        ["buildc-1"]
    );
    let _ = std::fs::remove_dir_all(root);
}
