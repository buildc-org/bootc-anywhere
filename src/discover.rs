use super::*;
use std::io::Write as _;

pub(crate) fn partitions(disk: &str) -> Result<Vec<Partition>, String> {
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
    partition_rows(&String::from_utf8_lossy(&out.stdout))
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

impl Drop for Mounts {
    fn drop(&mut self) {
        for at in &self.0 {
            let _ = Command::new("umount").arg(at).status();
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

fn old_systems(disk: &str, mounts: &mut Mounts) -> (Vec<OldSystem>, Vec<String>) {
    let mut systems = Vec::new();
    let mut unread = Vec::new();
    match mountables(disk) {
        Ok(devices) => {
            for (device, fstype) in devices {
                let at = match mount_ro(Path::new(&device), &fstype, mounts) {
                    Ok(at) => at,
                    Err(err) => {
                        unread.push(err);
                        continue;
                    }
                };
                let fstab = at.join("etc/fstab");
                if !fstab.is_file() {
                    continue;
                }
                let crypttab = at.join("etc/crypttab");
                systems.push(OldSystem {
                    crypttab: match crypttab.is_file() {
                        true => std::fs::read_to_string(&crypttab).unwrap_or_default(),
                        false => String::new(),
                    },
                    fstab: std::fs::read_to_string(&fstab).unwrap_or_default(),
                    at,
                });
            }
        }
        Err(err) => unread.push(err),
    }
    (systems, unread)
}

#[derive(Debug, PartialEq)]
pub(crate) struct OldKey {
    pub(crate) target: String,
    pub(crate) key: Key,
}

#[derive(Default)]
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

pub(crate) fn old_keys(disk: &str, partitions: &[Partition]) -> Discovered {
    if !partitions.iter().any(|part| part.fstype == "crypto_LUKS") {
        return Discovered::default();
    }
    let mut mounts = Mounts::default();
    let (systems, unread) = old_systems(disk, &mut mounts);
    discover(
        partitions,
        &systems,
        &unread,
        &mut mounts,
        &|container, key| test_key(&container.device, key),
    )
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
        .map(|cgroup| {
            cgroup.contains("bootc-anywhere.service")
                || cgroup.contains("bootc-anywhere-vt.service")
        })
        .unwrap_or(false)
}

pub(crate) fn console_vt() -> Option<u32> {
    let active = std::fs::read_to_string(tty0_active()).ok()?;
    active.trim().strip_prefix("tty")?.parse().ok()
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
