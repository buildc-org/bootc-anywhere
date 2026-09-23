use super::*;
use std::io::Write as _;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Slot {
    pub(crate) node: String,
    pub(crate) number: usize,
    pub(crate) start: u64,
    pub(crate) sectors: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DiskTable {
    pub(crate) first: u64,
    pub(crate) last: u64,
    pub(crate) sector: u64,
    pub(crate) label: String,
    pub(crate) slots: Vec<Slot>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DiskState {
    pub(crate) rdev: u64,
    pub(crate) dev: u64,
    pub(crate) ino: u64,
    pub(crate) table: DiskTable,
}

impl DiskTable {
    pub(crate) fn appendable(&self, deletes: &[String]) -> u64 {
        let gone = deleted_slots(deletes);
        let end = match self.cleared_by(deletes) {
            true => self.first,
            false => self
                .slots
                .iter()
                .filter(|slot| !gone.contains(&slot.number))
                .map(|slot| slot.start.saturating_add(slot.sectors))
                .max()
                .unwrap_or(self.first)
                .max(self.first),
        };
        self.last.saturating_add(1).saturating_sub(end)
    }

    fn cleared_by(&self, deletes: &[String]) -> bool {
        let gone = deleted_slots(deletes);
        self.slots.is_empty() || self.slots.iter().all(|slot| gone.contains(&slot.number))
    }

    pub(crate) fn wrong_label_for(&self, deletes: &[String], creates: usize) -> Option<String> {
        if creates == 0 {
            return None;
        }
        match self.label.is_empty() || self.label == "gpt" || self.cleared_by(deletes) {
            true => None,
            false => Some(copy::custom_not_gpt(&self.label)),
        }
    }

    pub(crate) fn appendable_gb(&self, deletes: &[String]) -> u64 {
        self.appendable(deletes)
            .saturating_mul(self.sector)
            .saturating_div(1_000_000_000)
    }

    pub(crate) fn surviving(&self, deletes: &[String]) -> Vec<usize> {
        let gone = deleted_slots(deletes);
        self.slots
            .iter()
            .filter(|slot| !gone.contains(&slot.number))
            .map(|slot| slot.number)
            .collect()
    }
}

fn deleted_slots(deletes: &[String]) -> Vec<usize> {
    deletes
        .iter()
        .filter_map(|device| partition_number(device).ok())
        .collect()
}

pub(crate) fn appended_slots(taken: &[usize], count: usize) -> Vec<usize> {
    let mut given: Vec<usize> = Vec::new();
    let mut number = 1;
    while given.len() < count {
        if !taken.contains(&number) {
            given.push(number);
        }
        number += 1;
    }
    given
}

#[cfg(test)]
pub(crate) fn table_of(dump: &str) -> DiskTable {
    let mut table = DiskTable::default();
    for line in dump.lines() {
        let line = line.trim();
        let number = |value: &str| value.trim().parse::<u64>().ok();
        if let Some(value) = line.strip_prefix("label:") {
            table.label = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("first-lba:") {
            table.first = number(value).unwrap_or_default();
        } else if let Some(value) = line.strip_prefix("last-lba:") {
            table.last = number(value).unwrap_or_default();
        } else if let Some(value) = line.strip_prefix("sector-size:") {
            table.sector = number(value).unwrap_or_default();
        } else if let Some((node, fields)) = line.split_once(" : ") {
            let node = node.trim().to_string();
            let Ok(at) = partition_number(&node) else {
                continue;
            };
            let field = |key: &str| {
                fields
                    .split(',')
                    .filter_map(|field| field.trim().strip_prefix(key))
                    .find_map(|value| value.trim().parse::<u64>().ok())
            };
            table.slots.push(Slot {
                node,
                number: at,
                start: field("start=").unwrap_or_default(),
                sectors: field("size=").unwrap_or_default(),
            });
        }
    }
    if table.sector == 0 {
        table.sector = 512;
    }
    table
}

fn device_path(disk: &str) -> String {
    match (cfg!(debug_assertions), disk.strip_prefix("/dev/")) {
        (true, Some(name)) => env::dev().join(name).to_string_lossy().into_owned(),
        _ => disk.to_string(),
    }
}

pub(crate) fn disk_table(disk: &str) -> Result<DiskTable, String> {
    fdisk::read_table(&device_path(disk))
}

#[cfg(test)]
pub(crate) fn sfdisk_table(disk: &str) -> Result<DiskTable, String> {
    let out = Command::new("sfdisk")
        .args(["--dump", disk])
        .env("LC_ALL", "C")
        .output()
        .map_err(|err| format!("sfdisk --dump {disk}: {err}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut table = match out.status.success() {
        true => table_of(&String::from_utf8_lossy(&out.stdout)),
        false if unpartitioned_dump(&stderr) => DiskTable {
            sector: 512,
            ..Default::default()
        },
        false => {
            return Err(format!("sfdisk --dump {disk}: {}", stderr.trim()));
        }
    };
    if table.last == 0 {
        table.first = 2048;
        table.last = disk_sectors(disk, table.sector).saturating_sub(34);
    }
    Ok(table)
}

#[cfg(test)]
fn unpartitioned_dump(stderr: &str) -> bool {
    stderr.contains("does not contain a recognized partition table")
}

pub(crate) fn disk_state(disk: &str) -> Result<DiskState, String> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::metadata(device_path(disk)).map_err(|err| format!("{disk}: {err}"))?;
    Ok(DiskState {
        rdev: metadata.rdev(),
        dev: metadata.dev(),
        ino: metadata.ino(),
        table: disk_table(disk)?,
    })
}

struct DiskLock(std::fs::File);

impl DiskLock {
    pub(crate) fn take(disk: &str) -> Result<Self, String> {
        use std::os::fd::AsRawFd as _;

        let file = std::fs::File::open(disk).map_err(|err| format!("{disk}: {err}"))?;
        // SAFETY: the file owns the descriptor and is not dropped until after the call, so the descriptor stays open for flock.
        let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if locked == 0 {
            Ok(Self(file))
        } else {
            Err(format!(
                "{disk} is busy changing its partition table; review it again"
            ))
        }
    }
}

impl Drop for DiskLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd as _;

        // SAFETY: self.0 is not dropped until after this call, so its descriptor stays open for the unlock.
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[cfg(test)]
pub(crate) fn disk_sectors(disk: &str, sector: u64) -> u64 {
    let sector = match sector {
        0 => 512,
        sector => sector,
    };
    let out = Command::new("lsblk")
        .args([
            "--bytes",
            "--nodeps",
            "--noheadings",
            "--output",
            "SIZE",
            disk,
        ])
        .output();
    let Ok(out) = out else { return 0 };
    if !out.status.success() {
        return std::fs::metadata(disk)
            .map(|at| at.len() / sector)
            .unwrap_or_default();
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .map(|bytes| bytes / sector)
        .unwrap_or_default()
}

pub(crate) fn created_devices(disk: &str, taken: &[usize], count: usize) -> Vec<String> {
    appended_slots(taken, count)
        .into_iter()
        .map(|number| fdisk::partname(disk, number))
        .collect()
}

pub(crate) fn drawn_slots(parts: &[Partition], deletes: &[String]) -> Vec<usize> {
    parts
        .iter()
        .filter(|part| !deletes.contains(&part.device))
        .filter_map(|part| partition_number(&part.device).ok())
        .collect()
}

pub(crate) fn created_type(target: &str) -> &'static str {
    match target {
        "/boot/efi" => "U",
        _ => "L",
    }
}

pub(crate) fn cut_partitions(layout: &mut CustomLayout) -> Result<(), String> {
    if layout.deletes.is_empty() && layout.creates.is_empty() {
        return Ok(());
    }
    let _lock = DiskLock::take(&layout.disk)?;
    let now = disk_state(&layout.disk)?;
    if layout.confirmed.as_ref() != Some(&now) {
        return Err(copy::table_changed(&layout.disk));
    }
    let numbers = apply_cuts(layout)?;
    for (create, number) in layout.creates.iter_mut().zip(numbers) {
        let Some(device) = settled_node(&layout.disk, number) else {
            return Err(format!(
                "partition {number} on {} was cut but no device node appeared \
                 for it; review the disk and install again",
                layout.disk
            ));
        };
        create.device = device;
    }
    Ok(())
}

pub(crate) fn apply_cuts(layout: &CustomLayout) -> Result<Vec<usize>, String> {
    if layout.deletes.is_empty() && layout.creates.is_empty() {
        return Ok(Vec::new());
    }
    let table = disk_table(&layout.disk)?;
    if let Some(why) = table.wrong_label_for(&layout.deletes, layout.creates.len()) {
        return Err(why);
    }
    let cleared = table.cleared_by(&layout.deletes);
    let numbers = match cleared {
        true => appended_slots(&[], layout.creates.len()),
        false => appended_slots(&table.surviving(&layout.deletes), layout.creates.len()),
    };
    let numbered = layout
        .deletes
        .iter()
        .map(|device| partition_number(device).map(|number| (device, number)))
        .collect::<Result<Vec<_>, _>>()?;
    match cleared {
        true => sfdisk_script(&layout.disk, &[], "label: gpt\n")?,
        false => {
            for (device, number) in numbered {
                let out = Command::new("sfdisk")
                    .args(["-q", "--delete", &layout.disk, &number.to_string()])
                    .output()
                    .map_err(|err| format!("sfdisk --delete: {err}"))?;
                if !out.status.success() {
                    return Err(format!(
                        "the cut could not remove {device}: {}\n\n{}",
                        String::from_utf8_lossy(&out.stderr).trim(),
                        copy::table_already_changed(&layout.disk)
                    ));
                }
            }
        }
    }
    if layout.creates.is_empty() {
        return Ok(Vec::new());
    }
    let script: String = layout
        .creates
        .iter()
        .map(|create| {
            format!(
                "size={}GB, type={}\n",
                create.gb,
                created_type(&create.target)
            )
        })
        .collect();
    sfdisk_script(&layout.disk, &["--append"], &script)?;
    let after = disk_table(&layout.disk)?;
    check_created_slots(layout, &numbers, &after)?;
    Ok(numbers)
}

pub(crate) fn check_created_slots(
    layout: &CustomLayout,
    numbers: &[usize],
    after: &DiskTable,
) -> Result<(), String> {
    for (create, number) in layout.creates.iter().zip(numbers) {
        let Some(slot) = after.slots.iter().find(|slot| slot.number == *number) else {
            return Err(format!(
                "partition {number} is not in the partition table the cut \
                 wrote; the disk numbered the new partitions differently than \
                 the screen drew them\n\n{}",
                copy::table_already_changed(&layout.disk)
            ));
        };
        let got = slot.sectors.saturating_mul(after.sector);
        let asked = create.gb.saturating_mul(1_000_000_000);
        if got < asked.saturating_sub(asked / 100) {
            return Err(format!(
                "partition {number} was cut at {} GB, though the plan asked \
                 for {} GB; sfdisk shrank it to fit and did not say so\n\n{}",
                got / 1_000_000_000,
                create.gb,
                copy::table_already_changed(&layout.disk)
            ));
        }
    }
    Ok(())
}

fn sfdisk_script(disk: &str, args: &[&str], script: &str) -> Result<(), String> {
    let mut command = Command::new("sfdisk");
    command.arg("-q");
    command.args(args);
    command.arg(disk);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("sfdisk {disk}: {err}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| format!("sfdisk {disk}: no stdin"))?
        .write_all(script.as_bytes())
        .map_err(|err| format!("sfdisk {disk}: {err}"))?;
    let out = child
        .wait_with_output()
        .map_err(|err| format!("sfdisk {disk}: {err}"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "the cut could not write the partition table on {disk}: {}\n\nthe \
             partition table on {disk} may already have been changed",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

const SETTLE: std::time::Duration = std::time::Duration::from_millis(100);
const SETTLE_TRIES: usize = 50;

pub(crate) fn settled_node(disk: &str, number: usize) -> Option<String> {
    for _ in 0..SETTLE_TRIES {
        let node = fdisk::partname(disk, number);
        if Path::new(&node).exists() {
            return Some(node);
        }
        std::thread::sleep(SETTLE);
    }
    None
}
