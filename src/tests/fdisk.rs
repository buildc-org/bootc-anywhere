use super::*;
use crate::fdisk::{partname, read_table};

fn gpt(name: &str, script: &str) -> Option<String> {
    let sfdisk = Command::new("sfdisk").arg("--version").output().ok()?;
    if !sfdisk.status.success() {
        return None;
    }
    let root = scratch(name);
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(8 * 1024 * 1024 * 1024))
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
        .write_all(script.as_bytes())
        .expect("the starting table");
    assert!(child.wait().expect("sfdisk").success());
    Some(disk)
}

#[test]
fn the_libfdisk_reader_agrees_with_the_sfdisk_reader() {
    let script = "label: gpt\nsize=20M, name=one\nsize=20M, name=two\nsize=20M, name=three\n";
    let Some(disk) = gpt("fdisk-agree", script) else {
        return;
    };
    let dumped = super::sfdisk_table(&disk).expect("the sfdisk table");
    let read = read_table(&disk).expect("the libfdisk table");
    assert_eq!(read, dumped);
}

#[test]
fn a_hole_and_a_reused_slot_read_the_same_through_both() {
    let script = "label: gpt\nsize=20M, name=one\nsize=20M, name=two\nsize=20M, name=three\n";
    let Some(disk) = gpt("fdisk-hole", script) else {
        return;
    };
    let mut child = Command::new("sfdisk")
        .args(["-q", "--delete", &disk, "2"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("sfdisk");
    assert!(child.wait().expect("sfdisk").success());

    let dumped = super::sfdisk_table(&disk).expect("the sfdisk table");
    let read = read_table(&disk).expect("the libfdisk table");
    assert_eq!(read, dumped);
    assert_eq!(read.slots.len(), 2);
    assert_eq!(super::appended_slots(&[1, 3], 1), [2]);
}

#[test]
fn an_unpartitioned_disk_reads_as_room_through_both() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("fdisk-blank");
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(200 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();

    let dumped = super::sfdisk_table(&disk).expect("the sfdisk table");
    let read = read_table(&disk).expect("the libfdisk table");
    assert_eq!(read, dumped);
    assert_eq!(read.label, "");
    assert_eq!(read.sector, 512);
    assert!(read.slots.is_empty());
    assert_eq!(read.last, 200 * 1024 * 2 - 34);
    assert_eq!(read.first, 2048);
}

#[test]
fn the_node_rule_has_a_part_branch_as_well_as_a_digit_branch() {
    assert_eq!(partname("/dev/vda", 1), "/dev/vda1");
    assert_eq!(partname("/dev/nvme0n1", 2), "/dev/nvme0n1p2");
    assert_eq!(partname("/dev/mmcblk0", 3), "/dev/mmcblk0p3");
    assert_eq!(partname("/dev/loop0", 1), "/dev/loop0p1");
    assert_eq!(
        partname("/dev/mapper/mydisk", 1),
        "/dev/mapper/mydisk-part1"
    );
    assert_eq!(
        partname("/dev/disk/by-id/ata-ST1000_ABC", 1),
        "/dev/disk/by-id/ata-ST1000_ABC-part1"
    );
    assert_eq!(
        partname("/dev/disk/by-path/pci-0000:00:1f.2-ata-1", 1),
        "/dev/disk/by-path/pci-0000:00:1f.2-ata-1-part1"
    );
    assert_eq!(
        partname("/dev/disk/by-uuid/abcd", 1),
        "/dev/disk/by-uuid/abcd1"
    );
    assert_eq!(
        partname("/dev/disk/by-partuuid/abcd", 1),
        "/dev/disk/by-partuuid/abcd1"
    );
    assert_eq!(
        partname("/dev/disk/by-uuid/abcd1234", 1),
        "/dev/disk/by-uuid/abcd1234p1"
    );
    for probe in ["/dev/mapper/pool21", "/dev/mapper/pool2p1"] {
        assert!(
            !std::path::Path::new(probe).exists(),
            "{probe} exists on this host, so the fallback is not what this case reads"
        );
    }
    assert_eq!(partname("/dev/mapper/pool2", 1), "/dev/mapper/pool2-part1");
}

#[test]
fn an_unreadable_device_is_a_refusal_and_not_an_empty_table() {
    let missing = scratch("fdisk-missing").join("absent.img");
    let err = read_table(&missing.to_string_lossy()).expect_err("no such device");
    assert!(err.contains("cannot read"), "{err}");
}

#[test]
fn a_dos_label_reads_the_conservative_span_through_both() {
    let script = "label: dos\nsize=20M\nsize=20M\n";
    let Some(disk) = gpt("fdisk-dos", script) else {
        return;
    };
    let dumped = super::sfdisk_table(&disk).expect("the sfdisk table");
    let read = read_table(&disk).expect("the libfdisk table");
    assert_eq!(read, dumped);
    assert_eq!(read.label, "dos");
    assert_eq!(read.first, 2048);
}

#[test]
fn a_dos_extended_container_is_a_slot_through_both() {
    let script = "label: dos\nsize=20M, type=83\ntype=5\n";
    let Some(disk) = gpt("fdisk-extended", script) else {
        return;
    };
    let dumped = super::sfdisk_table(&disk).expect("the sfdisk table");
    let read = read_table(&disk).expect("the libfdisk table");
    assert_eq!(read, dumped);
    let numbers: Vec<usize> = read.slots.iter().map(|slot| slot.number).collect();
    assert!(numbers.contains(&2), "the container is a slot: {numbers:?}");
}

fn ran(command: &mut Command) -> Option<String> {
    let out = command.output().ok()?;
    match out.status.success() {
        true => Some(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        false => None,
    }
}

struct Loop(String);

impl Drop for Loop {
    fn drop(&mut self) {
        let _ = Command::new("losetup").args(["-d", &self.0]).status();
    }
}

fn loop_4kn(name: &str, bytes: u64) -> (Loop, String) {
    assert_eq!(
        // SAFETY: libc::geteuid has no precondition, so the call is sound.
        unsafe { libc::geteuid() },
        0,
        "{name} needs root: a 4Kn loop device cannot be attached as a user"
    );
    assert!(
        ran(Command::new("losetup").arg("--version")).is_some(),
        "{name} needs losetup"
    );
    let root = scratch(name);
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(bytes))
        .expect("a backing file");
    let path = image.to_string_lossy().to_string();
    let device = ran(Command::new("losetup").args(["-b", "4096", "-f", "--show", &path]))
        .unwrap_or_else(|| panic!("{name}: losetup -b 4096 could not attach {path}"));
    (Loop(device), path)
}

#[test]
#[ignore = "needs root and losetup -b 4096"]
fn a_4kn_disk_with_no_label_reports_the_same_room_through_both() {
    let (device, _image) = loop_4kn("fdisk-4kn-blank", 512 * 1024 * 1024);
    let dumped = super::sfdisk_table(&device.0).expect("the sfdisk table");
    let read = read_table(&device.0).expect("the libfdisk table");

    assert_eq!(dumped.sector, 512, "the sfdisk reader falls back to 512");
    assert_eq!(read.sector, 4096, "libfdisk asks the device");
    let by_dump = dumped.appendable(&[]) * dumped.sector;
    let by_lib = read.appendable(&[]) * read.sector;
    assert!(
        by_dump >= by_lib,
        "the fallback reader must stay the optimistic side: {by_dump} against {by_lib}"
    );
    assert_eq!(
        by_dump - by_lib,
        33 * (read.sector - dumped.sector),
        "the reserve difference is not fully accounted for"
    );
}

#[test]
#[ignore = "needs root and losetup -b 4096"]
fn a_4kn_disk_with_a_gpt_reads_the_same_through_both() {
    let (device, _image) = loop_4kn("fdisk-4kn-gpt", 512 * 1024 * 1024);
    let script = "label: gpt\nsize=20M, name=one\nsize=20M, name=two\n";
    let mut child = Command::new("sfdisk")
        .args(["-q", &device.0])
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
    let wrote = child.wait().expect("sfdisk").success();
    let dumped = super::sfdisk_table(&device.0).expect("the sfdisk table");
    let read = read_table(&device.0).expect("the libfdisk table");

    assert!(wrote, "sfdisk wrote the starting table");
    assert_eq!(dumped.sector, 4096);
    assert_eq!(read, dumped);
}
