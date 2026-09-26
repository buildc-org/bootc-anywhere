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
fn the_free_regions_are_the_spans_the_surviving_partitions_leave() {
    let table = super::table_of(MEASURED_DUMP);
    let spans = |deletes: &[&str]| -> Vec<(u64, u64)> {
        table
            .regions(&deletes.iter().map(|at| at.to_string()).collect::<Vec<_>>())
            .into_iter()
            .map(|region| (region.start, region.sectors))
            .collect()
    };
    assert_eq!(spans(&[]), [(43008, 40960), (145408, 264159)]);
    assert_eq!(spans(&["/dev/vda2"]), [(43008, 40960), (124928, 284639)]);
    assert_eq!(spans(&["/dev/vda1"]), [(2048, 81920), (145408, 264159)]);
    let all: Vec<String> = table.slots.iter().map(|slot| slot.node.clone()).collect();
    let all: Vec<&str> = all.iter().map(String::as_str).collect();
    assert_eq!(spans(&all), [(2048, 407519)]);
}

#[test]
fn a_create_takes_the_lowest_region_with_room_for_it() {
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
                start: 10 * gb + 2048,
                sectors: 20 * gb,
            },
            Slot {
                node: "/dev/vda3".to_string(),
                number: 3,
                start: 30 * gb + 2048,
                sectors: 10 * gb,
            },
        ],
    };
    let create = |gb: u64, offset: u64| Created {
        gb,
        offset,
        target: "/".to_string(),
        fstype: "btrfs".to_string(),
        ..Default::default()
    };
    let deletes = vec!["/dev/vda2".to_string()];
    let (starts, _) = place_creates(&table, &deletes, &[create(5, 0), create(50, 0)])
        .expect("both creates fit the disk");
    let hole = table.aligned(&Region {
        start: 10 * gb + 2048,
        sectors: 20 * gb,
    });
    let tail = table.aligned(&Region {
        start: 40 * gb + 2048,
        sectors: 60 * gb - 2047,
    });
    assert_eq!(starts, [hole.start, tail.start]);
    let (starts, _) = place_creates(&table, &deletes, &[create(5, 3)]).expect("a 5 GB create");
    assert_eq!(starts, [hole.start + 3 * gb]);
    let room = create_rooms(&table, &[], &[]);
    assert_eq!(room.largest, 59);
    assert_eq!(place_creates(&table, &[], &[create(60, 0)]).err(), Some(59));
}

#[test]
fn the_create_window_opens_on_the_room_the_plan_leaves() {
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
                start: 10 * gb + 2048,
                sectors: 20 * gb,
            },
            Slot {
                node: "/dev/vda3".to_string(),
                number: 3,
                start: 30 * gb + 2048,
                sectors: 10 * gb,
            },
        ],
    };
    assert_eq!(create_rooms(&table, &[], &[]).first, 59);
    let deletes = vec!["/dev/vda2".to_string()];
    let rooms = create_rooms(&table, &deletes, &[]);
    assert_eq!(rooms.first, 19, "the freed hole is the default size");
    assert_eq!(rooms.largest, 59, "the tail is still the largest region");
    let held = [Created {
        gb: 5,
        ..Default::default()
    }];
    assert_eq!(create_rooms(&table, &deletes, &held).first, 14);
    let too_big = [Created {
        gb: 95,
        ..Default::default()
    }];
    let rooms = create_rooms(&table, &deletes, &too_big);
    assert_eq!(rooms.first, 0);
    assert!(rooms.largest < 95, "{}", rooms.largest);
}

#[test]
fn the_create_size_opens_on_the_first_region_not_the_smallest() {
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
                sectors: 30 * gb,
            },
            Slot {
                node: "/dev/vda2".to_string(),
                number: 2,
                start: 30 * gb + 2048,
                sectors: 8 * gb,
            },
            Slot {
                node: "/dev/vda3".to_string(),
                number: 3,
                start: 60 * gb,
                sectors: 30 * gb,
            },
        ],
    };
    let rooms = create_rooms(&table, &[], &[]);
    assert_eq!(rooms.first, 21, "the first region in sector order");
    assert_eq!(rooms.largest, 21);
    let placed = place_creates(
        &table,
        &[],
        &[Created {
            gb: rooms.first,
            ..Default::default()
        }],
    );
    let (start, _) = placed.expect("the default size fits the first region");
    let first = table.aligned(&Region {
        start: 38 * gb + 2048,
        sectors: 22 * gb - 2048,
    });
    assert_eq!(start, [first.start]);
}

#[test]
fn a_region_is_trimmed_to_the_alignment_sfdisk_writes() {
    let table = super::table_of(MEASURED_DUMP);
    let region = super::Region {
        start: 43008,
        sectors: 40960,
    };
    assert_eq!(
        table.aligned(&region),
        super::Region {
            start: 43008,
            sectors: 40960
        }
    );
    let off = super::Region {
        start: 43009,
        sectors: 40958,
    };
    assert_eq!(
        table.aligned(&off),
        super::Region {
            start: 45056,
            sectors: 36864
        }
    );
    assert_eq!(
        table.aligned(&super::Region {
            start: 43009,
            sectors: 100
        }),
        super::Region {
            start: 45056,
            sectors: 0
        }
    );
    assert_eq!(table.placeable_gb(&region), 40960 * 512 / 1_000_000_000);
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
fn a_delete_matches_its_slot_when_lsblk_and_sfdisk_name_it_differently() {
    // The free regions cannot follow file order because the dump lists part3 before part2.
    let dump = "label: gpt\n\
                first-lba: 2048\n\
                last-lba: 409566\n\
                sector-size: 512\n\
                /dev/disk/by-id/ata-X-part1 : start=2048, size=20480\n\
                /dev/disk/by-id/ata-X-part2 : start=124928, size=20480\n\
                /dev/disk/by-id/ata-X-part3 : start=22528, size=20480\n";
    let table = super::table_of(dump);
    assert_eq!(table.slots.len(), 3);
    let deletes = vec!["/dev/sda3".to_string()];
    assert_eq!(table.surviving(&deletes), [1, 2]);
    let highest = vec!["/dev/sda2".to_string()];
    assert_eq!(
        table.regions(&highest),
        [super::Region {
            start: 43008,
            sectors: 366559
        }]
    );
    assert_ne!(table.regions(&highest), table.regions(&[]));
    assert_eq!(
        super::created_devices("/dev/disk/by-id/ata-X", &table.surviving(&deletes), 1),
        ["/dev/disk/by-id/ata-X-part3"]
    );
    let all = vec![
        "/dev/sda1".to_string(),
        "/dev/sda2".to_string(),
        "/dev/sda3".to_string(),
    ];
    assert_eq!(table.surviving(&all), [] as [usize; 0]);
    assert_eq!(
        table.regions(&all),
        [super::Region {
            start: 2048,
            sectors: 407519
        }]
    );
}

#[test]
fn a_created_device_with_no_node_beside_it_takes_the_part_branch() {
    for probe in [
        "/dev/mapper/mydisk4",
        "/dev/mapper/mydiskp4",
        "/dev/mapper/mydisk5",
        "/dev/mapper/mydiskp5",
        "/dev/disk/by-id/wwn-0x50004",
        "/dev/disk/by-id/wwn-0x5000p4",
    ] {
        assert!(
            !std::path::Path::new(probe).exists(),
            "{probe} exists on this host, so the fallback is not what this case reads"
        );
    }
    let table = super::table_of(MEASURED_DUMP);
    assert_eq!(
        super::created_devices("/dev/mapper/mydisk", &table.surviving(&[]), 2),
        ["/dev/mapper/mydisk-part4", "/dev/mapper/mydisk-part5"]
    );
    assert_eq!(
        super::created_devices("/dev/disk/by-id/wwn-0x5000", &table.surviving(&[]), 1),
        ["/dev/disk/by-id/wwn-0x5000-part4"]
    );
}

#[test]
fn a_created_esp_is_cut_as_an_efi_partition() {
    let at = |target: &str| Created {
        target: target.to_string(),
        ..Default::default()
    };
    assert_eq!(super::created_type(&at("/boot/efi")), "U");
    assert_eq!(super::created_type(&at("/")), "L");
    assert_eq!(super::created_type(&at("")), "L");
}

#[test]
fn a_delete_the_cut_cannot_number_takes_nothing_off_the_disk() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("cut-unnamed-delete");
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(200 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let script = "label: gpt
size=20M, name=one
size=20M, name=two
";
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
        deletes: vec![format!("{disk}1"), format!("{disk}-spare")],
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let err = super::apply_cuts(&layout).expect_err("a delete naming no number");
    assert!(err.contains("names no partition number"), "{err}");
    assert_eq!(super::disk_table(&disk).expect("the table"), before);

    let overflowed = CustomLayout {
        deletes: vec![format!("{disk}1"), format!("{disk}99999999999999999999")],
        ..layout.clone()
    };
    let err = super::apply_cuts(&overflowed).expect_err("a delete that overflows");
    assert!(err.contains("names no partition number"), "{err}");
    assert_eq!(super::disk_table(&disk).expect("the table"), before);

    let cleared = CustomLayout {
        deletes: vec![
            format!("{disk}1"),
            format!("{disk}2"),
            format!("{disk}-spare"),
        ],
        ..layout.clone()
    };
    let err = super::apply_cuts(&cleared).expect_err("a delete naming no number");
    assert!(err.contains("names no partition number"), "{err}");
    assert_eq!(super::disk_table(&disk).expect("the table"), before);
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
    let script = "label: gpt\nsize=2G, name=one\nsize=2G, name=two\nsize=2G, name=three\n";
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
    let deleted = before
        .slots
        .iter()
        .find(|slot| slot.number == 2)
        .expect("the middle slot")
        .clone();
    let layout = CustomLayout {
        disk: disk.clone(),
        deletes: vec![format!("{disk}2")],
        creates: vec![
            Created {
                gb: 1,
                target: "/boot/efi".to_string(),
                fstype: "fat32".to_string(),
                device: String::new(),
                ..Default::default()
            },
            Created {
                gb: 1,
                target: "/".to_string(),
                fstype: "btrfs".to_string(),
                device: String::new(),
                ..Default::default()
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
    assert_eq!(cut, [2, 4], "the cut must return the slots the screen drew");

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
    assert_eq!(
        two.start, deleted.start,
        "the first create took the deleted slot's own sector"
    );
    let four = after
        .slots
        .iter()
        .find(|slot| slot.number == 4)
        .expect("slot 4");
    assert!(four.start >= two.start + two.sectors, "the creates overlap");
    assert!(
        four.start + four.sectors <= three.start,
        "the creates ran past the hole into the surviving partition"
    );
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
fn a_create_bigger_than_the_free_region_stops_the_install() {
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
    let before = super::disk_table(&disk).expect("the table");
    let layout = CustomLayout {
        disk: disk.clone(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let err = super::apply_cuts(&layout).expect_err("a 1 GB partition on a 200 MiB disk");
    assert!(err.contains("no room"), "{err}");
    assert_eq!(
        super::disk_table(&disk).expect("the table"),
        before,
        "the refusal left the disk untouched"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_cut_writes_the_planned_names() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("cut-names");
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(8 * 1024 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let script = "label: gpt\nsize=1G, name=one\nsize=1G, name=two\n";
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

    let layout = CustomLayout {
        disk: disk.clone(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            label: "root".to_string(),
            device: String::new(),
            ..Default::default()
        }],
        renames: vec![Rename {
            partition: format!("{disk}1"),
            label: "renamed".to_string(),
        }],
        ..Default::default()
    };
    let cut = super::apply_cuts(&layout).expect("the cut");
    assert_eq!(cut, [3]);
    let dump = Command::new("sfdisk")
        .args(["--dump", &disk])
        .output()
        .expect("a dump");
    let dump = String::from_utf8_lossy(&dump.stdout);
    for (number, label) in [(1, "renamed"), (3, "root")] {
        let line = dump
            .lines()
            .find(|line| line.starts_with(&format!("{disk}{number} ")))
            .unwrap_or_else(|| panic!("no line for slot {number} in:\n{dump}"));
        assert!(
            line.contains(&format!("name=\"{label}\"")),
            "slot {number}: {line}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_created_slot_missing_from_the_read_back_table_stops_the_install() {
    let disk = "/dev/vda";
    let layout = CustomLayout {
        disk: disk.to_string(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let err = super::check_created_slots(&layout, &[1], &DiskTable::default())
        .expect_err("a missing sfdisk slot must stop the install");
    assert!(err.contains(&copy::table_already_changed(disk)), "{err}");
}

#[test]
fn a_created_slot_is_checked_by_number_when_the_node_name_differs() {
    let layout = CustomLayout {
        disk: "/dev/mapper/mydisk".to_string(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let after = DiskTable {
        label: "gpt".to_string(),
        sector: 512,
        slots: vec![Slot {
            node: "/dev/mapper/mydisk2".to_string(),
            number: 2,
            start: 40960,
            sectors: 1_000_000_000 / 512,
        }],
        ..Default::default()
    };
    assert_eq!(super::check_created_slots(&layout, &[2], &after), Ok(()));
    let short = DiskTable {
        slots: vec![Slot {
            node: "/dev/mapper/mydisk2".to_string(),
            number: 2,
            start: 40960,
            sectors: 1_000_000 / 512,
        }],
        ..after.clone()
    };
    let err = super::check_created_slots(&layout, &[2], &short)
        .expect_err("a slot sfdisk shrank must stop the install");
    assert!(err.contains("did not say so"), "{err}");
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
        filled.wrong_label_for(&[], 1, 0).as_deref(),
        Some(copy::custom_not_gpt("dos").as_str())
    );
    assert_eq!(
        filled.wrong_label_for(&[], 0, 1).as_deref(),
        Some(copy::custom_not_gpt("dos").as_str())
    );
    assert!(filled.wrong_label_for(&[], 0, 0).is_none());
    let all = vec!["/dev/vda1".to_string()];
    assert!(filled.wrong_label_for(&all, 1, 1).is_none());
    assert_eq!(
        filled.regions(&all),
        [super::Region {
            start: 2048,
            sectors: 407519
        }]
    );
    let blank = DiskTable {
        first: 2048,
        last: 409566,
        sector: 512,
        ..Default::default()
    };
    assert!(blank.wrong_label_for(&[], 1, 0).is_none());
    assert_eq!(
        blank.regions(&[]),
        [super::Region {
            start: 2048,
            sectors: 407519
        }]
    );
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
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let freed = vec!["/dev/vda2".to_string()];
    let rooms = create_rooms(&table, &freed, &[]);
    assert_eq!(rooms.first, 89, "the create window's size default");
    assert_eq!(rooms.largest, 89);
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
        Some(copy::custom_too_big(87).as_str())
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
            ..Default::default()
        },
        Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
            ..Default::default()
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
        assert_eq!(cut, [1, 2], "{name}");
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
        let table = DiskTable {
            first: 2048,
            last: super::disk_sectors(&disk, sector).saturating_sub(34),
            sector,
            label: String::new(),
            slots: Vec::new(),
        };
        let region = table.regions(&[]);
        table.placeable_gb(&region[0])
    };
    assert_eq!(room(512), room(4096));
    assert_eq!(room(512), 8);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_existing_node_is_taken_at_once() {
    let root = scratch("settled-node");
    let disk = root.join("disk.img");
    std::fs::File::create(&disk).expect("the disk");
    let node = root.join("disk.img1");
    std::fs::File::create(&node).expect("the node");
    assert_eq!(
        super::settled_node(&disk.to_string_lossy(), 1),
        Some(node.to_string_lossy().into_owned())
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_cut_whose_node_never_appears_stops_the_install() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("cut-node-wait");
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(8 * 1024 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let mut layout = CustomLayout {
        disk: disk.clone(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
            ..Default::default()
        }],
        ..Default::default()
    };
    layout.confirmed = Some(super::disk_state(&disk).expect("the disk state"));
    let err = super::cut_partitions(&mut layout).expect_err("no node grows on a file");
    assert!(err.contains("no device node appeared"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_layout_that_cuts_nothing_writes_no_partition_table() {
    let mut layout = CustomLayout::empty("/dev/definitely-not-a-disk");
    assert!(super::cut_partitions(&mut layout).is_ok());
}

fn three_slots() -> DiskTable {
    let sector = 512u64;
    let gb = 1_000_000_000 / sector;
    let slot = |number: usize, start: u64, sectors: u64| Slot {
        node: format!("/dev/vda{number}"),
        number,
        start,
        sectors,
    };
    DiskTable {
        first: 2048,
        last: 100 * gb,
        sector,
        label: "gpt".to_string(),
        slots: vec![
            slot(1, 2048, 10 * gb),
            slot(2, 10 * gb + 2048, 20 * gb),
            slot(3, 30 * gb + 2048, 10 * gb),
        ],
    }
}

fn names() -> Vec<(usize, String)> {
    vec![(1, "efi".to_string()), (3, "vda3".to_string())]
}

#[test]
fn a_freed_hole_is_bounded_by_the_partitions_beside_it() {
    let deletes = vec!["/dev/vda2".to_string()];
    let holes = create_holes(&three_slots(), &deletes, &[], &names());
    let sides: Vec<(Option<&str>, Option<&str>)> = holes
        .iter()
        .map(|hole| (hole.before.as_deref(), hole.after.as_deref()))
        .collect();
    assert_eq!(sides, [(Some("efi"), Some("vda3")), (Some("vda3"), None)]);
}

#[test]
fn a_planned_create_bounds_the_region_it_leaves() {
    let deletes = vec!["/dev/vda2".to_string()];
    let held = [Created {
        gb: 5,
        ..Default::default()
    }];
    let mut names = names();
    names.push((2, "new".to_string()));
    let holes = create_holes(&three_slots(), &deletes, &held, &names);
    assert_eq!(holes[0].before.as_deref(), Some("new"));
    assert_eq!(holes[0].after.as_deref(), Some("vda3"));
}
