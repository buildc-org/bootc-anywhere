use super::*;
use std::io::Write as _;

pub(crate) fn partitions(disk: &str) -> Result<(Vec<Partition>, bool), String> {
    let out = Command::new("lsblk")
        .args([
            "--json",
            "--paths",
            "--output",
            "NAME,SIZE,FSTYPE,LABEL,TYPE,PARTTYPE,UUID",
            disk,
        ])
        .output()
        .map_err(|err| format!("lsblk {disk}: {err}"))?;
    if !out.status.success() {
        return Err(format!(
            "lsblk {disk}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    Ok((partition_rows(&raw)?, carries_content(&raw)?))
}

pub(crate) fn carries_content(raw: &str) -> Result<bool, String> {
    let doc = Json::parse(raw).map_err(|err| format!("lsblk wrote invalid JSON: {err}"))?;
    let Some(disk) = json::items(&doc, "blockdevices").first() else {
        return Ok(false);
    };
    Ok(
        json::text(disk, "fstype").is_some_and(|fstype| !fstype.is_empty())
            || !json::items(disk, "children").is_empty(),
    )
}

pub(crate) fn partition_rows(raw: &str) -> Result<Vec<Partition>, String> {
    fn visit(node: &Json, found: &mut Vec<Partition>) {
        if json::text(node, "type").as_deref() == Some("part") {
            found.push(Partition {
                device: json::text(node, "name").unwrap_or_default(),
                size: json::text(node, "size").unwrap_or_default(),
                fstype: json::text(node, "fstype").unwrap_or_default(),
                label: json::text(node, "label").unwrap_or_default(),
                parttype: json::text(node, "parttype").unwrap_or_default(),
                uuid: json::text(node, "uuid").unwrap_or_default(),
            });
        }
        for child in json::items(node, "children") {
            visit(child, found);
        }
    }

    let doc = Json::parse(raw).map_err(|err| format!("lsblk wrote invalid JSON: {err}"))?;
    let mut found = Vec::new();
    for block in json::items(&doc, "blockdevices") {
        visit(block, &mut found);
    }
    Ok(found)
}

#[derive(Debug)]
pub(crate) struct OldSystem {
    pub(crate) at: PathBuf,
    pub(crate) crypttab: String,
    pub(crate) fstab: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum KeySource {
    Inside(PathBuf),
    Default,
    OnDevice { device: String, path: PathBuf },
    Waited,
    Unreadable,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Crypttab {
    pub(crate) name: String,
    pub(crate) device: String,
    pub(crate) key: KeySource,
}

fn device_spec(said: &str) -> bool {
    said.starts_with("/dev/")
        || ["UUID=", "PARTUUID=", "LABEL="]
            .iter()
            .any(|prefix| said.starts_with(prefix))
}

fn on_device(said: &str, passdev: bool) -> Option<KeySource> {
    let (first, rest) = said.split_once(':')?;
    let (device, path, timed) = match (device_spec(first), passdev) {
        (true, _) => (first, rest, true),
        (false, false) if device_spec(rest) => (rest, first, false),
        _ => return None,
    };
    let path = match timed {
        true => path.split_once(':').map_or(path, |(path, _timeout)| path),
        false => path,
    };
    Some(KeySource::OnDevice {
        device: device.to_string(),
        path: PathBuf::from(path),
    })
}

pub(crate) fn crypttabs(raw: &str) -> Vec<Crypttab> {
    let mut found = Vec::new();
    for line in raw.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        let (Some(name), Some(device)) = (words.next(), words.next()) else {
            continue;
        };
        let field = words.next().unwrap_or("none");
        let options = words.next().unwrap_or("");
        let keyscript = options
            .split(',')
            .find_map(|option| option.strip_prefix("keyscript="));
        let timed = options
            .split(',')
            .any(|option| option.starts_with("keyfile-timeout="));
        let key = match keyscript {
            Some(script) if script.rsplit('/').next() == Some("passdev") => {
                on_device(field, true).unwrap_or(KeySource::Unreadable)
            }
            Some(_) => KeySource::Unreadable,
            None if field == "none" || field == "-" => KeySource::Default,
            None => match on_device(field, false) {
                Some(device) => device,
                None if timed => KeySource::Waited,
                None => KeySource::Inside(PathBuf::from(field)),
            },
        };
        found.push(Crypttab {
            name: name.to_string(),
            device: device.to_string(),
            key,
        });
    }
    found
}

pub(crate) fn names(entry: &Crypttab, container: &Partition) -> bool {
    let said = entry.device.as_str();
    if let Some(uuid) = said.strip_prefix("UUID=") {
        return !container.uuid.is_empty() && uuid.eq_ignore_ascii_case(&container.uuid);
    }
    if let Some(uuid) = said.strip_prefix("/dev/disk/by-uuid/") {
        return !container.uuid.is_empty() && uuid.eq_ignore_ascii_case(&container.uuid);
    }
    said == container.device
}

fn fstab_target(raw: &str, name: &str) -> String {
    let mapper = format!("/dev/mapper/{name}");
    for line in raw.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        let (Some(device), Some(target)) = (words.next(), words.next()) else {
            continue;
        };
        if device == mapper {
            return target.to_string();
        }
    }
    String::new()
}

const MOUNTS: &str = "/run/tect-old";

fn mounts_root() -> PathBuf {
    std::env::var_os("BUILDC_MOUNT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(MOUNTS))
}

#[derive(Default)]
pub(crate) struct Mounts(Vec<PathBuf>);

impl Mounts {
    pub(crate) fn release(&mut self, at: &Path) -> Result<(), String> {
        let out = Command::new("umount")
            .arg(at)
            .output()
            .map_err(|err| format!("umount {}: {err}", at.display()))?;
        if !out.status.success() {
            return Err(format!(
                "unmounting {}: {}",
                at.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        self.0.retain(|mounted| mounted != at);
        Ok(())
    }
}

impl Mounts {
    pub(crate) fn mount(&mut self, device: &str, at: &Path) -> Result<(), String> {
        std::fs::create_dir_all(at).map_err(|err| format!("{}: {err}", at.display()))?;
        let out = Command::new("mount")
            .arg(device)
            .arg(at)
            .output()
            .map_err(|err| format!("mount: {err}, and it is what writes {device}"))?;
        if !out.status.success() {
            return Err(format!(
                "mounting {device}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        self.0.push(at.to_path_buf());
        Ok(())
    }
}

impl Drop for Mounts {
    fn drop(&mut self) {
        for at in self.0.iter().rev() {
            if let Err(why) = Command::new("umount")
                .arg(at)
                .output()
                .map_err(|err| err.to_string())
                .and_then(|out| match out.status.success() {
                    true => Ok(()),
                    false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                })
            {
                eprintln!("{PROGRAM}: {} is still mounted: {why}", at.display());
            }
        }
    }
}

pub(crate) fn mount_options(fstype: &str) -> &'static str {
    // A read-only mount still replays the journal of a Linux filesystem, so the scan forbids that write to the old disk.
    match LINUX_FILESYSTEMS.contains(&fstype) {
        true => "ro,norecovery",
        false => "ro",
    }
}

fn mount_ro(device: &Path, fstype: &str, mounts: &mut Mounts) -> Result<PathBuf, String> {
    let at = mounts_root().join(format!("old-{}", mounts.0.len() + 1));
    std::fs::create_dir_all(&at).map_err(|err| format!("{}: {err}", at.display()))?;
    let options = mount_options(fstype);
    let out = Command::new("mount")
        .args(["-o", options])
        .arg(device)
        .arg(&at)
        .output()
        .map_err(|err| format!("mount: {err}, and it is what reads {}", device.display()))?;
    if !out.status.success() {
        return Err(format!(
            "mounting {} read-only: {}",
            device.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    mounts.0.push(at.clone());
    Ok(at)
}

pub(crate) fn mount_rw(device: &str, mounts: &mut Mounts) -> Result<PathBuf, String> {
    let at = mounts_root().join(format!("write-{}", mounts.0.len() + 1));
    mounts.mount(device, &at)?;
    Ok(at)
}

pub(crate) fn mountable_rows(raw: &str) -> Result<Vec<(String, String)>, String> {
    fn visit(node: &Json, found: &mut Vec<(String, String)>) {
        let kind = json::text(node, "type").unwrap_or_default();
        let fstype = json::text(node, "fstype").unwrap_or_default();
        if ["part", "crypt", "lvm"].contains(&kind.as_str())
            && !fstype.is_empty()
            && fstype != "crypto_LUKS"
        {
            found.push((json::text(node, "name").unwrap_or_default(), fstype));
        }
        for child in json::items(node, "children") {
            visit(child, found);
        }
    }
    let doc = Json::parse(raw).map_err(|err| format!("lsblk wrote invalid JSON: {err}"))?;
    let mut found = Vec::new();
    for block in json::items(&doc, "blockdevices") {
        visit(block, &mut found);
    }
    Ok(found)
}

fn mountables(disk: &str) -> Result<Vec<(String, String)>, String> {
    let out = Command::new("lsblk")
        .args(["--json", "--paths", "--output", "NAME,FSTYPE,TYPE", disk])
        .output()
        .map_err(|err| format!("lsblk {disk}: {err}"))?;
    if !out.status.success() {
        return Err(format!(
            "lsblk {disk}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    mountable_rows(&String::from_utf8_lossy(&out.stdout))
}

#[derive(Default)]
struct Walked {
    systems: Vec<OldSystem>,
    unread: Vec<String>,
    labels: Vec<(String, Vec<Carried>)>,
}

fn walk_disk(disk: &str, mounts: &mut Mounts) -> Walked {
    let mut walked = Walked::default();
    let devices = match mountables(disk) {
        Ok(devices) => devices,
        Err(err) => {
            walked.unread.push(err);
            return walked;
        }
    };
    for (device, fstype) in devices {
        let at = match mount_ro(Path::new(&device), &fstype, mounts) {
            Ok(at) => at,
            Err(err) => {
                walked.unread.push(err);
                continue;
            }
        };
        let labels = labels_at(&at, &fstype);
        if !labels.is_empty() {
            walked.labels.push((device.clone(), labels));
        }
        let fstab = at.join("etc/fstab");
        if !fstab.is_file() {
            continue;
        }
        let crypttab = at.join("etc/crypttab");
        walked.systems.push(OldSystem {
            crypttab: match crypttab.is_file() {
                true => std::fs::read_to_string(&crypttab).unwrap_or_default(),
                false => String::new(),
            },
            fstab: std::fs::read_to_string(&fstab).unwrap_or_default(),
            at,
        });
    }
    walked
}

pub(crate) fn labels_at(at: &Path, fstype: &str) -> Vec<Carried> {
    match is_fat(fstype) {
        true => esp::entries(at),
        false => boot_labels(at),
    }
}

pub(crate) fn inside(mount: &Path, name: &Path) -> Option<PathBuf> {
    // The installer treats a mounted old system as untrusted, so a path it names is resolved inside the mount and refused when it leaves.
    let root = std::fs::canonicalize(mount).ok()?;
    let said = std::fs::canonicalize(mount.join(name.strip_prefix("/").unwrap_or(name))).ok()?;
    said.starts_with(&root).then_some(said)
}

pub(crate) fn inside_file(mount: &Path, name: &Path) -> Option<PathBuf> {
    inside(mount, name).filter(|path| path.is_file())
}

fn boot_labels(at: &Path) -> Vec<Carried> {
    os_release_name(at)
        .or_else(|| loader_title(at))
        .map(|name| Carried {
            name,
            links: Vec::new(),
        })
        .into_iter()
        .collect()
}

fn os_release_name(at: &Path) -> Option<String> {
    let file = inside_file(at, Path::new("etc/os-release"))?;
    let text = std::fs::read_to_string(&file).ok()?;
    pretty_name(&text)
}

fn pretty_name(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let value = line
            .strip_prefix("PRETTY_NAME=")?
            .trim()
            .trim_matches(['"', '\'']);
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// Reads the title of the first loader entry a mounted filesystem carries.
/// The entries sit under `loader/` where this is the boot partition and under
/// `boot/loader/` where it is the root.
fn loader_title(at: &Path) -> Option<String> {
    for file in loader_files(at) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        if let Some(title) = entry_title(&text) {
            return Some(title);
        }
    }
    None
}

fn entry_title(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("title "))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
}

/// The three GPT types a Windows install carries: the Microsoft reserved
/// partition, the basic data volume and the recovery partition.
const MSR: &str = "e3c9e316-0b5c-4db8-817d-f92df00215ae";
const BASIC_DATA: &str = "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7";
const RECOVERY: &str = "de94bba4-06d1-4d40-a16a-bfd50179d6ac";

/// A detected system's name is data read off the disk rather than screen
const WINDOWS: &str = "Windows";

pub(crate) fn windows_label(partitions: &[Partition]) -> Option<(String, String)> {
    let carried = |kind: &str| {
        partitions
            .iter()
            .find(|part| part.parttype.eq_ignore_ascii_case(kind))
    };
    let reserved = carried(MSR)?;
    carried(BASIC_DATA).or_else(|| carried(RECOVERY))?;
    let named = carried(BASIC_DATA).unwrap_or(reserved);
    Some((named.device.clone(), WINDOWS.to_string()))
}

pub(crate) fn windows_link(own: Option<&str>, installs: &[String]) -> Option<String> {
    own.map(str::to_string).or_else(|| match installs {
        [only] => Some(only.clone()),
        _ => None,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OldKey {
    pub(crate) target: String,
    pub(crate) key: Key,
}

#[derive(Clone, Default)]
pub(crate) struct Discovered {
    pub(crate) found: Vec<(String, OldKey)>,
    pub(crate) why: Vec<(String, String)>,
}

impl Discovered {
    pub(crate) fn key(&self, partition: &str) -> Option<&Key> {
        self.found
            .iter()
            .find(|(device, _)| device == partition)
            .map(|(_, old)| &old.key)
    }
}

pub(crate) fn key_file(volume: &str, root: &Path, path: &Path) -> Result<Vec<u8>, String> {
    let said = root.join(path.strip_prefix("/").unwrap_or(path));
    let file = std::fs::canonicalize(&said).map_err(|err| {
        copy::old_root_key_missing(volume, &path.display().to_string(), &err.to_string())
    })?;
    let at = std::fs::canonicalize(root).map_err(|err| format!("{}: {err}", root.display()))?;
    if !file.starts_with(&at) || !file.is_file() {
        return Err(copy::old_root_key_outside(&path.display().to_string()));
    }
    std::fs::read(&file).map_err(|err| {
        copy::old_root_key_missing(volume, &path.display().to_string(), &err.to_string())
    })
}

fn key_bytes(system: &OldSystem, entry: &Crypttab, mounts: &mut Mounts) -> Result<Vec<u8>, String> {
    match &entry.key {
        KeySource::Inside(path) => key_file(&entry.name, &system.at, path),
        // systemd-cryptsetup falls back to this key file when the crypttab line names no key.
        KeySource::Default => key_file(
            &entry.name,
            &system.at,
            &PathBuf::from(format!("/etc/cryptsetup-keys.d/{}.key", entry.name)),
        ),
        KeySource::OnDevice { device, path } => {
            let resolved = resolve_device(device)?;
            if removable(&resolved) {
                return Err(copy::old_root_key_removable(&entry.name));
            }
            let fstype = fstype_of(&resolved);
            let at = mount_ro(&resolved, &fstype, mounts)?;
            key_file(&entry.name, &at, path)
        }
        KeySource::Waited => Err(copy::old_root_key_waited(&entry.name)),
        KeySource::Unreadable => Err(copy::old_root_key_unreadable(&entry.name)),
    }
}

fn resolve_device(said: &str) -> Result<PathBuf, String> {
    if said.starts_with('/') {
        return Ok(PathBuf::from(said));
    }
    let Some((tag, value)) = ["UUID", "LABEL", "PARTUUID"].iter().find_map(|tag| {
        said.strip_prefix(&format!("{tag}="))
            .map(|value| (*tag, value))
    }) else {
        return Err(format!("{said} is not a device"));
    };
    let out = Command::new("blkid")
        .args(["-t", &format!("{tag}={value}"), "-o", "device"])
        .output()
        .map_err(|err| format!("blkid: {err}, and it is what resolves {said}"))?;
    labelled(&String::from_utf8_lossy(&out.stdout))
        .into_iter()
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{said} is not present"))
}

fn fstype_of(device: &Path) -> String {
    Command::new("blkid")
        .args(["-s", "TYPE", "-o", "value"])
        .arg(device)
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default()
}

const SYS_CLASS_BLOCK: &str = "/sys/class/block";

pub(crate) fn removable_at(class: &Path, device: &Path) -> bool {
    let device = std::fs::canonicalize(device).unwrap_or_else(|_| device.to_path_buf());
    let Some(name) = device.file_name() else {
        return false;
    };
    let Ok(at) = std::fs::canonicalize(class.join(name)) else {
        return false;
    };
    let disk = match at.join("partition").exists() {
        true => match at.parent() {
            Some(parent) => parent.to_path_buf(),
            None => at,
        },
        false => at,
    };
    std::fs::read_to_string(disk.join("removable")).is_ok_and(|flag| flag.trim() == "1")
}

pub(crate) fn removable(device: &Path) -> bool {
    removable_at(Path::new(SYS_CLASS_BLOCK), device)
}

pub(crate) fn test_key(container: &str, key: &[u8]) -> Result<bool, String> {
    let mut command = Command::new("cryptsetup");
    command
        .args(["-q", "luksOpen", "--test-passphrase", "--key-file", "-"])
        .arg(container)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|err| format!("cryptsetup: {err}, and it is what tests a key"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(key);
    }
    let status = child.wait().map_err(|err| format!("cryptsetup: {err}"))?;
    Ok(status.success())
}

pub(crate) fn discover(
    partitions: &[Partition],
    systems: &[OldSystem],
    unread: &[String],
    mounts: &mut Mounts,
    opens: &dyn Fn(&Partition, &[u8]) -> Result<bool, String>,
) -> Discovered {
    let mut found = Discovered::default();
    for partition in partitions
        .iter()
        .filter(|part| part.fstype == "crypto_LUKS")
    {
        let mut named = false;
        let mut why = None;
        let mut key = None;
        'systems: for system in systems {
            for entry in crypttabs(&system.crypttab) {
                if !names(&entry, partition) {
                    continue;
                }
                named = true;
                match key_bytes(system, &entry, mounts)
                    .and_then(|bytes| opens(partition, &bytes).map(|opens| (bytes, opens)))
                {
                    Ok((bytes, true)) => {
                        key = Some(OldKey {
                            target: fstab_target(&system.fstab, &entry.name),
                            key: Key::Data(bytes),
                        });
                        break 'systems;
                    }
                    Ok((_, false)) => why = Some(copy::old_root_key_wrong(&partition.device)),
                    Err(err) => why = Some(err),
                }
            }
        }
        match key {
            Some(key) => found.found.push((partition.device.clone(), key)),
            None => {
                let why = why.unwrap_or_else(|| match named {
                    true => copy::old_root_key_wrong(&partition.device),
                    false if !unread.is_empty() => copy::old_root_partly(&unread[0]),
                    false if !systems.is_empty() => copy::old_root_unnamed(&partition.device),
                    false => copy::OLD_ROOT_NONE.to_string(),
                });
                found.why.push((partition.device.clone(), why));
            }
        }
    }
    found
}

pub(crate) struct DiskScan {
    pub(crate) device: String,
    pub(crate) detail: String,
    pub(crate) partitions: Vec<Partition>,
    pub(crate) carries: bool,
    pub(crate) table: Result<DiskTable, String>,
    pub(crate) labels: Vec<(String, Vec<Label>)>,
    pub(crate) keys: Discovered,
}

#[derive(Default)]
pub(crate) struct Scan {
    pub(crate) disks: Vec<DiskScan>,
    pub(crate) unread: Vec<(String, String)>,
}

impl Scan {
    pub(crate) fn read() -> Self {
        let in_use = in_use_now();
        let mut held = Mounts::default();
        let mut listed = Vec::new();
        let mut unread = Vec::new();
        for (device, detail) in disks(&sys_block(), &in_use) {
            let (partitions, carries) = match partitions(&device) {
                Ok(parts) => parts,
                Err(why) => {
                    unread.push((device, why));
                    continue;
                }
            };
            let table = disk_table(&device);
            let walked = walk_disk(&device, &mut held);
            listed.push((device, detail, partitions, carries, table, walked));
        }
        let all: Vec<Partition> = listed
            .iter()
            .flat_map(|(_, _, partitions, ..)| partitions.clone())
            .collect();
        let installs: Vec<String> = listed
            .iter()
            .filter_map(|(_, _, partitions, ..)| windows_label(partitions))
            .map(|(device, _)| device)
            .collect();
        let mut found = Vec::new();
        for (device, detail, partitions, carries, table, walked) in listed {
            let own = windows_label(&partitions);
            let windows = windows_link(own.as_ref().map(|(device, _)| device.as_str()), &installs);
            let mut labels = esp::linked(walked.labels, &all, windows.as_deref());
            if let Some((at, name)) = own {
                labels.push((
                    at,
                    vec![Label {
                        name,
                        link: windows.unwrap_or_default(),
                    }],
                ));
            }
            let keys = discover(
                &partitions,
                &walked.systems,
                &walked.unread,
                &mut held,
                &|container, key| test_key(&container.device, key),
            );
            found.push(DiskScan {
                device,
                detail,
                partitions,
                carries,
                table,
                labels,
                keys,
            });
        }
        Self {
            disks: found,
            unread,
        }
    }

    pub(crate) fn get(&self, device: &str) -> Option<&DiskScan> {
        self.disks.iter().find(|disk| disk.device == device)
    }

    pub(crate) fn refusal(&self, device: &str) -> Option<String> {
        let named = std::fs::canonicalize(device).unwrap_or_else(|_| PathBuf::from(device));
        if let Some((_, why)) = self.unread.iter().find(|(at, _)| Path::new(at) == named) {
            return Some(why.clone());
        }
        match self.disks.is_empty() {
            true => self.unread.first().map(|(_, why)| why.clone()),
            false => None,
        }
    }

    pub(crate) fn table(&self, device: &str) -> Result<DiskTable, String> {
        match self.get(device) {
            Some(disk) => disk.table.clone(),
            None => disk_table(device),
        }
    }

    pub(crate) fn only_empty_disk(&self) -> Option<String> {
        match self.disks.as_slice() {
            [only] if self.unread.is_empty() && only.partitions.is_empty() && !only.carries => {
                Some(only.device.clone())
            }
            _ => None,
        }
    }
}

pub(crate) fn power_off() -> Result<(), String> {
    let out = Command::new("systemctl")
        .arg("poweroff")
        .output()
        .map_err(|err| format!("systemctl: {err}, and it is what stops a machine"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "systemctl poweroff: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

pub(crate) fn under_media_installer() -> bool {
    std::fs::read_to_string("/proc/self/cgroup")
        .map(|cgroup| media_installer(&cgroup))
        .unwrap_or(false)
}

fn media_installer(cgroup: &str) -> bool {
    cgroup.contains("bootc-anywhere.service") || cgroup.contains("bootc-anywhere-vt.service")
}

pub(crate) fn console_vt() -> Option<u32> {
    let active = std::fs::read_to_string(tty0_active()).ok()?;
    active_vt(&active)
}

fn active_vt(active: &str) -> Option<u32> {
    active.trim().strip_prefix("tty")?.parse().ok()
}

#[cfg(feature = "fuzzing")]
pub(crate) fn fuzz_readers(text: &str) {
    let _ = partition_rows(text);
    let _ = carries_content(text);
    let _ = crypttabs(text);
    let _ = fstab_target(text, "fuzz");
    let _ = mountable_rows(text);
    let _ = pretty_name(text);
    let _ = entry_title(text);
    let _ = media_installer(text);
    let _ = active_vt(text);
}

pub(crate) fn switch_to_shell() -> Result<(), String> {
    let shell = match console_vt() {
        Some(1) | None => 2,
        Some(vt) => {
            if vt == 2 {
                3
            } else {
                2
            }
        }
    };
    let out = Command::new("chvt")
        .arg(shell.to_string())
        .output()
        .map_err(|err| format!("chvt: {err}, and it is what switches to the shell"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "chvt: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}
