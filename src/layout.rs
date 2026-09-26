use super::*;

pub(crate) const ESP_GB: u64 = 2;
pub(crate) const BOOT_GB: u64 = 2;

pub(crate) const SYSROOT: &str = "/run/buildc-sysroot";

pub(crate) struct Prepared {
    #[expect(dead_code, reason = "the guard is held for its drop")]
    pub(crate) mounts: Mounts,
    #[expect(dead_code, reason = "the guard is held for its drop")]
    pub(crate) opened: Mappers,
    pub(crate) layout: CustomLayout,
    pub(crate) recovery: Option<String>,
}

pub(crate) fn prepare(payload: &Payload, answers: &Answers) -> Result<Prepared, String> {
    release_stale(&answers.disk)?;
    let mut layout = match &answers.layout {
        Some(layout) => layout.clone(),
        None => whole_disk(&answers.disk, payload, &answers.encryption)?,
    };
    for (fstype, target) in planned_formats(&layout, &payload.filesystem) {
        let program = mkfs_args(fstype, target)?[0];
        if !on_path(program) {
            return Err(copy::no_mkfs(program, fstype));
        }
    }
    if answers.layout.is_some() && found_by_type(payload) {
        let label = disk_state(&layout.disk)?.table.label;
        if label != "gpt" {
            return Err(copy::root_type_needs_gpt(&layout.disk));
        }
    }
    let changes_table = layout.changes_table();
    cut_partitions(&mut layout)?;
    let disk = layout.disk.clone();
    written(payload, answers, layout).map_err(|why| match changes_table {
        true => format!("{why}\n\n{}", copy::table_already_changed(&disk)),
        false => why,
    })
}

fn written(
    payload: &Payload,
    answers: &Answers,
    mut layout: CustomLayout,
) -> Result<Prepared, String> {
    esp::remove(&layout)?;
    let recovery = seal(&mut layout, &answers.encryption)?;
    let opened = open_volumes(Some(&layout))?;
    let created_typed = layout
        .creates
        .iter()
        .any(|create| create.target == "/" && create.discoverable);
    if found_by_type(payload) && !created_typed {
        retag_root(&layout)?;
    }
    let volumes = volumes(&layout, &payload.filesystem);
    for volume in &volumes {
        if let Some(fstype) = &volume.format {
            make_filesystem(fstype, volume)?;
        }
    }
    let mounts = mount_volumes(&volumes)?;
    Ok(Prepared {
        mounts,
        opened,
        layout,
        recovery,
    })
}

pub(crate) fn whole_disk(
    disk: &str,
    payload: &Payload,
    encryption: &Encryption,
) -> Result<CustomLayout, String> {
    let boot = match payload.bootloader.as_str() {
        "grub2" => true,
        "systemd" => false,
        other => return Err(copy::unknown_bootloader(other)),
    };
    if payload.filesystem.is_empty() {
        return Err(copy::NO_ROOT_FILESYSTEM.to_string());
    }
    let state = disk_state(disk)?;
    let deletes: Vec<String> = state
        .table
        .slots
        .iter()
        .map(|slot| slot.node.clone())
        .collect();
    let mut creates = vec![Created {
        gb: ESP_GB,
        target: "/boot/efi".to_string(),
        fstype: "fat32".to_string(),
        label: "EFI-SYSTEM".to_string(),
        ..Default::default()
    }];
    if boot {
        creates.push(Created {
            gb: BOOT_GB,
            target: "/boot".to_string(),
            fstype: "ext4".to_string(),
            label: "boot".to_string(),
            ..Default::default()
        });
    }
    let rest = create_rooms(&state.table, &deletes, &creates).largest;
    let reserve = payload.reserve_gb();
    if rest < reserve {
        return Err(copy::custom_root_too_small(rest, reserve));
    }
    creates.push(Created {
        gb: rest,
        target: "/".to_string(),
        fstype: payload.filesystem.clone(),
        label: "root".to_string(),
        encrypt: encryption.kind != NONE,
        discoverable: found_by_type(payload),
        ..Default::default()
    });
    Ok(CustomLayout {
        disk: disk.to_string(),
        deletes,
        creates,
        confirmed: Some(state),
        ..Default::default()
    })
}

fn planned_formats<'a>(
    layout: &'a CustomLayout,
    root_fs: &'a str,
) -> impl Iterator<Item = (&'a str, &'a str)> {
    layout
        .mounts
        .iter()
        .filter(|mount| mount.fstype != "unformatted")
        .map(|mount| (mount.fstype.as_str(), mount.target.as_str()))
        .chain(
            layout
                .creates
                .iter()
                .map(|create| (create.fstype.as_str(), create.target.as_str())),
        )
        .chain(
            layout
                .opens
                .iter()
                .filter(|open| open.target == "/")
                .map(move |_| (root_fs, "/")),
        )
}

pub(crate) fn mkfs_args(fstype: &str, target: &str) -> Result<&'static [&'static str], String> {
    match (fstype, target) {
        ("ext4", "/") => Ok(&["mkfs.ext4", "-F", "-O", "verity"]),
        ("ext4", _) => Ok(&["mkfs.ext4", "-F"]),
        ("xfs", _) => Ok(&["mkfs.xfs", "-f"]),
        ("btrfs", _) => Ok(&["mkfs.btrfs", "-f"]),
        (fat, _) if is_fat(fat) => Ok(&["mkfs.fat", "-F", "32"]),
        (other, _) => Err(copy::unformattable(other, target)),
    }
}

fn found_by_type(payload: &Payload) -> bool {
    !payload.boot.is_empty() || (payload.composefs && payload.bootloader == "systemd")
}

pub(crate) fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

fn make_filesystem(fstype: &str, volume: &Volume) -> Result<(), String> {
    let args = mkfs_args(fstype, &volume.target)?;
    let device = &volume.device;
    let out = Command::new(args[0])
        .args(&args[1..])
        .arg(device)
        .output()
        .map_err(|err| format!("{}: {err}, and it is what formats {device}", args[0]))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "{} could not format {device}: {}",
            args[0],
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

pub(crate) fn seal(
    layout: &mut CustomLayout,
    encryption: &Encryption,
) -> Result<Option<String>, String> {
    let sealed: Vec<(String, String)> = layout
        .creates
        .iter()
        .filter(|create| create.encrypt)
        .map(|create| (create.device.clone(), create.target.clone()))
        .collect();
    if sealed.is_empty() {
        return Ok(None);
    }
    let (key, recovery) = match Encryption::wants_passphrase(&encryption.kind) {
        true => (encryption.passphrase.clone(), None),
        false => {
            let key = String::from_utf8_lossy(&random_key()?).into_owned();
            (key.clone(), Some(key))
        }
    };
    for (partition, target) in sealed {
        luks_format(&partition, key.as_bytes())?;
        layout.opens.push(LuksOpen {
            partition,
            target,
            key: Key::Passphrase(key.clone()),
        });
    }
    Ok(recovery)
}

#[derive(Debug, PartialEq)]
pub(crate) struct Volume {
    pub(crate) device: String,
    pub(crate) target: String,
    pub(crate) format: Option<String>,
}

pub(crate) fn volumes(layout: &CustomLayout, root_fs: &str) -> Vec<Volume> {
    let mounts = layout.mounts.iter().map(|mount| Volume {
        device: mount.partition.clone(),
        target: mount.target.clone(),
        format: (mount.fstype != "unformatted").then(|| mount.fstype.clone()),
    });
    let creates = layout
        .creates
        .iter()
        .filter(|create| !create.encrypt)
        .map(|create| Volume {
            device: create.device.clone(),
            target: create.target.clone(),
            format: Some(create.fstype.clone()),
        });
    let opens = layout.mappers().into_iter().map(|(name, open)| Volume {
        device: mapper_path(&name),
        target: open.target.clone(),
        format: layout
            .creates
            .iter()
            .find(|create| create.encrypt && create.device == open.partition)
            .map(|create| create.fstype.clone())
            .or_else(|| (open.target == "/").then(|| root_fs.to_string())),
    });
    mounts.chain(creates).chain(opens).collect()
}

pub(crate) fn mount_volumes(volumes: &[Volume]) -> Result<Mounts, String> {
    let mut mounts = Mounts::default();
    for volume in mount_order(volumes) {
        let at = Path::new(SYSROOT).join(volume.target.trim_start_matches('/'));
        mounts.mount(&volume.device, &at)?;
    }
    Ok(mounts)
}

pub(crate) fn mount_order(volumes: &[Volume]) -> Vec<&Volume> {
    let mut placed: Vec<&Volume> = volumes
        .iter()
        .filter(|volume| volume.target.starts_with('/') && volume.target != "/swap")
        .collect();
    placed.sort_by_key(|volume| volume.target.len());
    placed
}

fn release_stale(disk: &str) -> Result<(), String> {
    let mounted = Command::new("mountpoint")
        .args(["-q", SYSROOT])
        .status()
        .map_err(|err| format!("mountpoint: {err}, and it is what finds a left install"))?;
    if mounted.success() {
        let out = Command::new("umount")
            .args(["-R", SYSROOT])
            .output()
            .map_err(|err| format!("umount {SYSROOT}: {err}"))?;
        if !out.status.success() {
            return Err(format!(
                "an earlier install is still mounted at {SYSROOT}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    for name in stale_mappers(Path::new("/sys/block"), disk)? {
        close_volume(&name)?;
    }
    Ok(())
}

pub(crate) fn stale_mappers(blocks: &Path, disk: &str) -> Result<Vec<String>, String> {
    let wanted = Path::new(disk)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let entries =
        std::fs::read_dir(blocks).map_err(|err| format!("{}: {err}", blocks.display()))?;
    let mut stale = Vec::new();
    for entry in entries {
        let at = entry
            .map_err(|err| format!("{}: {err}", blocks.display()))?
            .path();
        let Ok(name) = std::fs::read_to_string(at.join("dm/name")) else {
            continue;
        };
        let name = name.trim();
        // The `buildc-` prefix marks a mapper this installer opened, so another tool's mapper is left alone.
        if !name.starts_with("buildc-") {
            continue;
        }
        let slaves = std::fs::read_dir(at.join("slaves"))
            .map_err(|err| format!("{}: {err}", at.join("slaves").display()))?;
        for slave in slaves {
            let slave = slave.map_err(|err| format!("{}: {err}", at.display()))?;
            let parent = std::fs::canonicalize(slave.path())
                .map_err(|err| format!("{}: {err}", slave.path().display()))?;
            let on_disk = parent
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|holder| holder.to_string_lossy() == wanted);
            if on_disk {
                stale.push(name.to_string());
                break;
            }
        }
    }
    Ok(stale)
}
