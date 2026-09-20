use super::*;

#[test]
fn the_table_reader_takes_the_sectors_sfdisk_reports() {
    let table = super::table_of(MEASURED_DUMP);
    assert_eq!(table.first, 2048);
    assert_eq!(table.last, 409566);
    assert_eq!(table.sector, 512);
    assert_eq!(table.slots.len(), 3);
    assert_eq!(table.slots[1].node, "/dev/vda2");
    assert_eq!(table.slots[1].number, 2);
    assert_eq!(table.slots[1].start, 124928);
    assert_eq!(table.slots[1].sectors, 20480);
    assert_eq!(super::table_of("").sector, 512);
}

#[test]
fn appendable_room_is_the_tail_and_never_a_hole() {
    let table = super::table_of(MEASURED_DUMP);
    assert_eq!(table.appendable(&[]), 409566 + 1 - 145408);
    let without = vec!["/dev/vda2".to_string()];
    assert_eq!(table.appendable(&without), 409566 + 1 - 124928);
    let middle = vec!["/dev/vda1".to_string()];
    assert_eq!(table.appendable(&middle), table.appendable(&[]));
    let all: Vec<String> = table.slots.iter().map(|slot| slot.node.clone()).collect();
    assert_eq!(table.appendable(&all), 409566 + 1 - 2048);
}

#[test]
fn appendable_gb_counts_in_decimal_gb() {
    let table = super::table_of(MEASURED_DUMP);
    assert_eq!(
        table.appendable_gb(&[]),
        (409566 + 1 - 145408) * 512 / 1_000_000_000
    );
    assert_eq!(table.appendable_gb(&[]), 0);
}

#[test]
fn a_created_partition_takes_the_lowest_free_slot() {
    let table = super::table_of(MEASURED_DUMP);
    let after = |deletes: &[&str]| {
        table.surviving(&deletes.iter().map(|at| at.to_string()).collect::<Vec<_>>())
    };
    assert_eq!(super::appended_slots(&after(&["/dev/vda2"]), 1), vec![2]);
    assert_eq!(super::appended_slots(&after(&[]), 2), vec![4, 5]);
    assert_eq!(
        super::appended_slots(&after(&["/dev/vda1", "/dev/vda3"]), 3),
        vec![1, 3, 4]
    );
    assert_eq!(
        super::appended_slots(&after(&["/dev/vda1", "/dev/vda2", "/dev/vda3"]), 3),
        vec![1, 2, 3]
    );
}

#[test]
fn the_screen_and_the_cut_agree_on_a_created_node() {
    let table = super::table_of(MEASURED_DUMP);
    let parts: Vec<Partition> = ["/dev/vda1", "/dev/vda2", "/dev/vda3"]
        .into_iter()
        .map(|device| Partition {
            device: device.to_string(),
            ..Default::default()
        })
        .collect();
    for deletes in [
        vec![],
        vec!["/dev/vda2".to_string()],
        vec!["/dev/vda1".to_string(), "/dev/vda3".to_string()],
    ] {
        assert_eq!(
            super::drawn_slots(&parts, &deletes),
            table.surviving(&deletes),
            "deletes {deletes:?}"
        );
    }
}

#[test]
fn a_created_device_follows_the_slot_it_will_get() {
    let table = super::table_of(MEASURED_DUMP);
    assert_eq!(
        super::created_devices("/dev/vda", &table.surviving(&[]), 2),
        ["/dev/vda4", "/dev/vda5"]
    );
    let gone = vec!["/dev/vda1".to_string()];
    assert_eq!(
        super::created_devices("/dev/vda", &table.surviving(&gone), 2),
        ["/dev/vda1", "/dev/vda4"]
    );
    assert_eq!(
        super::created_devices("/dev/nvme0n1", &table.surviving(&gone), 2),
        ["/dev/nvme0n1p1", "/dev/nvme0n1p4"]
    );
}

#[test]
fn a_created_esp_is_cut_as_an_efi_partition() {
    assert_eq!(super::created_type("/boot/efi"), "U");
    assert_eq!(super::created_type("/"), "L");
    assert_eq!(super::created_type(""), "L");
}

#[test]
fn the_cut_writes_the_table_the_screen_drew() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("cut-table");
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(8 * 1024 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let script = "label: gpt\nsize=20M, name=one\nsize=20M, name=two\nsize=20M, name=three\n";
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
        .write_all(script.as_bytes())
        .expect("the starting table");
    assert!(child.wait().expect("sfdisk").success());

    let before = super::disk_table(&disk).expect("the table");
    let layout = CustomLayout {
        disk: disk.clone(),
        deletes: vec![format!("{disk}2")],
        creates: vec![
            Created {
                gb: 1,
                target: "/boot/efi".to_string(),
                fstype: "fat32".to_string(),
                device: String::new(),
            },
            Created {
                gb: 1,
                target: "/".to_string(),
                fstype: "btrfs".to_string(),
                device: String::new(),
            },
        ],
        ..Default::default()
    };
    let drew = super::created_devices(
        &disk,
        &before.surviving(&layout.deletes),
        layout.creates.len(),
    );
    assert_eq!(drew, [format!("{disk}2"), format!("{disk}4")]);

    let cut = super::apply_cuts(&layout).expect("the cut");
    assert_eq!(cut, drew, "the cut must return the nodes the screen drew");

    let after = super::disk_table(&disk).expect("the table after");
    let numbers: Vec<usize> = {
        let mut numbers: Vec<usize> = after.slots.iter().map(|slot| slot.number).collect();
        numbers.sort_unstable();
        numbers
    };
    assert_eq!(numbers, [1, 2, 3, 4], "slots after the cut: {numbers:?}");
    let two = after
        .slots
        .iter()
        .find(|slot| slot.number == 2)
        .expect("slot 2");
    assert!(
        two.sectors * after.sector >= 1_000_000_000,
        "slot 2 is {} sectors, not the created GB",
        two.sectors
    );
    let three = after
        .slots
        .iter()
        .find(|slot| slot.number == 3)
        .expect("slot 3");
    for number in [2, 4] {
        let slot = after
            .slots
            .iter()
            .find(|slot| slot.number == number)
            .expect("a created slot");
        assert!(
            slot.start >= three.start + three.sectors,
            "slot {number} starts at {}, inside or before the surviving table",
            slot.start
        );
    }
    let dump = Command::new("sfdisk")
        .args(["--dump", &disk])
        .output()
        .expect("a dump");
    let dump = String::from_utf8_lossy(&dump.stdout);
    let esp = dump
        .lines()
        .find(|line| line.starts_with(&format!("{disk}2 ")))
        .expect("the ESP line");
    assert!(
        esp.to_uppercase()
            .contains("C12A7328-F81F-11D2-BA4B-00A0C93EC93B"),
        "{esp}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_partition_sfdisk_had_to_shrink_stops_the_install() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("cut-clamp");
    let image = root.join("small.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(200 * 1024 * 1024))
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
        .write_all(b"label: gpt\n")
        .expect("an empty table");
    assert!(child.wait().expect("sfdisk").success());
    let layout = CustomLayout {
        disk: disk.clone(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
        }],
        ..Default::default()
    };
    let err = super::apply_cuts(&layout).expect_err("a 1 GB partition on a 200 MiB disk");
    assert!(err.contains("did not say so"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_created_node_missing_from_the_read_back_table_stops_the_install() {
    let disk = "/dev/vda";
    let layout = CustomLayout {
        disk: disk.to_string(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
        }],
        ..Default::default()
    };
    let err = super::check_created_nodes(&layout, &[format!("{disk}1")], &DiskTable::default())
        .expect_err("a missing sfdisk node must stop the install");
    assert!(err.contains(&copy::table_already_changed(disk)), "{err}");
}

#[test]
fn a_disk_with_no_gpt_span_still_has_room() {
    let dos = super::table_of(
            "label: dos\nunit: sectors\nsector-size: 512\n\n/dev/vda1 : start=2048, size=40960, type=83\n",
        );
    assert_eq!(dos.label, "dos");
    assert_eq!(dos.last, 0, "a dos dump carries no last-lba");
    let filled = DiskTable {
        first: 2048,
        last: 409566,
        sector: 512,
        label: "dos".to_string(),
        slots: dos.slots.clone(),
    };
    assert_eq!(
        filled.wrong_label_for(&[], 1).as_deref(),
        Some(copy::custom_not_gpt("dos").as_str())
    );
    assert!(filled.wrong_label_for(&[], 0).is_none());
    let all = vec!["/dev/vda1".to_string()];
    assert!(filled.wrong_label_for(&all, 1).is_none());
    assert_eq!(filled.appendable(&all), 409566 + 1 - 2048);
    let blank = DiskTable {
        first: 2048,
        last: 409566,
        sector: 512,
        ..Default::default()
    };
    assert!(blank.wrong_label_for(&[], 1).is_none());
    assert_eq!(blank.appendable(&[]), 409566 + 1 - 2048);
}

#[test]
fn a_plan_that_outgrew_its_room_is_refused_before_the_cut() {
    let sector = 512u64;
    let gb = 1_000_000_000 / sector;
    let table = DiskTable {
        first: 2048,
        last: 100 * gb,
        sector,
        label: "gpt".to_string(),
        slots: vec![
            Slot {
                node: "/dev/vda1".to_string(),
                number: 1,
                start: 2048,
                sectors: 10 * gb,
            },
            Slot {
                node: "/dev/vda2".to_string(),
                number: 2,
                start: 2048 + 10 * gb,
                sectors: 90 * gb - 2048,
            },
        ],
    };
    let plan = |deletes: Vec<&str>, creates: Vec<(u64, &str, &str)>| CustomLayout {
        disk: "/dev/vda".to_string(),
        deletes: deletes.iter().map(|at| at.to_string()).collect(),
        creates: creates
            .into_iter()
            .map(|(gb, target, fstype)| Created {
                gb,
                target: target.to_string(),
                fstype: fstype.to_string(),
                device: String::new(),
            })
            .collect(),
        ..Default::default()
    };
    assert_eq!(table.appendable_gb(&["/dev/vda2".to_string()]), 89);
    let ok = plan(
        vec!["/dev/vda2"],
        vec![(2, "/boot/efi", "fat32"), (87, "/", "btrfs")],
    );
    assert_eq!(layout_short_of(&ok, false, Some(&table), 0), None);
    let over = plan(
        vec!["/dev/vda2"],
        vec![(2, "/boot/efi", "fat32"), (88, "/", "btrfs")],
    );
    assert_eq!(
        layout_short_of(&over, false, Some(&table), 0).as_deref(),
        Some(copy::custom_too_big(89).as_str())
    );
    let mut restored = ok.clone();
    restored.deletes.clear();
    assert_eq!(
        layout_short_of(&restored, false, Some(&table), 0).as_deref(),
        Some(copy::custom_too_big(0).as_str())
    );
    let together = plan(
        vec!["/dev/vda1", "/dev/vda2"],
        vec![(60, "/", "btrfs"), (60, "/boot/efi", "fat32")],
    );
    assert!(layout_short_of(&together, false, Some(&table), 0).is_some());
}

#[test]
fn a_blank_and_a_dos_disk_are_cut_as_gpt() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("cut-fresh-label");
    let creates = vec![
        Created {
            gb: 1,
            target: "/boot/efi".to_string(),
            fstype: "fat32".to_string(),
            device: String::new(),
        },
        Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
        },
    ];
    for (name, start, deletes) in [
        ("blank.img", "", vec![]),
        ("dos.img", "label: dos\nsize=20M\n", vec![1usize]),
    ] {
        let image = root.join(name);
        std::fs::File::create(&image)
            .and_then(|file| file.set_len(8 * 1024 * 1024 * 1024))
            .expect("a backing file");
        let disk = image.to_string_lossy().to_string();
        if !start.is_empty() {
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
                .write_all(start.as_bytes())
                .expect("the starting table");
            assert!(child.wait().expect("sfdisk").success());
        }
        let layout = CustomLayout {
            disk: disk.clone(),
            deletes: deletes.iter().map(|at| format!("{disk}{at}")).collect(),
            creates: creates.clone(),
            ..Default::default()
        };
        let cut = super::apply_cuts(&layout).unwrap_or_else(|err| panic!("cutting {name}: {err}"));
        assert_eq!(cut, [format!("{disk}1"), format!("{disk}2")], "{name}");
        let after = super::disk_table(&disk).expect("the table after");
        assert_eq!(after.label, "gpt", "{name} came out {}", after.label);
        assert_eq!(after.slots.len(), 2, "{name}");
        let dump = Command::new("sfdisk")
            .args(["--dump", &disk])
            .output()
            .expect("a dump");
        let dump = String::from_utf8_lossy(&dump.stdout);
        let esp = dump
            .lines()
            .find(|line| line.starts_with(&format!("{disk}1 ")))
            .unwrap_or_else(|| panic!("no ESP line for {name} in:\n{dump}"));
        assert!(
            esp.to_uppercase()
                .contains("C12A7328-F81F-11D2-BA4B-00A0C93EC93B"),
            "{name}: {esp}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_derived_span_counts_in_the_disks_own_sectors() {
    let root = scratch("disk-sectors");
    let image = root.join("disk.img");
    let bytes = 8u64 * 1024 * 1024 * 1024;
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(bytes))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    assert_eq!(super::disk_sectors(&disk, 512), bytes / 512);
    assert_eq!(super::disk_sectors(&disk, 4096), bytes / 4096);
    assert_eq!(super::disk_sectors(&disk, 0), bytes / 512);
    let room = |sector: u64| {
        DiskTable {
            first: 2048,
            last: super::disk_sectors(&disk, sector).saturating_sub(34),
            sector,
            label: String::new(),
            slots: Vec::new(),
        }
        .appendable_gb(&[])
    };
    assert_eq!(room(512), room(4096));
    assert_eq!(room(512), 8);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_node_that_exists_settles_at_once() {
    assert!(super::settled(Path::new("/dev/null")));
}

#[test]
fn a_layout_that_cuts_nothing_writes_no_partition_table() {
    let mut layout = CustomLayout::empty("/dev/definitely-not-a-disk");
    assert!(super::cut_partitions(&mut layout).is_ok());
}
