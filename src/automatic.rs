use super::*;

const PCR_POLICY: &str = "/usr/share/secureboot/pcr-policy.pem";

pub(crate) const CREDENTIAL: &str = "cryptsetup.passphrase";
pub(crate) const CREDENTIAL_DIR: &str = "loader/credentials";

pub(crate) const SLOT_FILE: &str = "cryptsetup.slot";

pub(crate) struct Ready {
    pub(crate) image: String,
    pub(crate) disk: String,
    pub(crate) partition: String,
    pub(crate) key: Key,
    pub(crate) pin: bool,
}

pub(crate) enum Offered {
    Ready(Ready),
    Unavailable(String),
    None,
}

pub(crate) fn offered(payload: &Payload, answers: &Answers, recovery: Option<&str>) -> Offered {
    let Some((key, pin)) = unlocking_key(answers, recovery) else {
        return Offered::None;
    };
    if payload.boot.is_empty() {
        return Offered::Unavailable(copy::AUTO_NO_STUB.to_string());
    }
    let partition = match encrypted_root(answers) {
        Ok(partition) => partition,
        Err(why) => return Offered::Unavailable(why),
    };
    if !pcr_policy_in(&payload.image) {
        return Offered::Unavailable(copy::AUTO_NO_POLICY.to_string());
    }
    Offered::Ready(Ready {
        image: payload.image.clone(),
        disk: answers.disk.clone(),
        partition,
        key,
        pin,
    })
}

pub(crate) fn unlocking_key(answers: &Answers, recovery: Option<&str>) -> Option<(Key, bool)> {
    if let Some(layout) = &answers.layout {
        if answers.opened != Opened::Tpm2 {
            return None;
        }
        let open = layout.opens.iter().find(|open| open.target == "/")?;
        return Some((open.key.clone(), false));
    }
    match answers.encryption.kind.as_str() {
        "tpm2-luks" | "tpm2-luks-pin" => recovery.map(|key| {
            (
                Key::Passphrase(key.to_string()),
                answers.encryption.kind.ends_with("pin"),
            )
        }),
        "tpm2-luks-passphrase" => Some((
            Key::Passphrase(answers.encryption.passphrase.clone()),
            false,
        )),
        _ => None,
    }
}

pub(crate) fn encrypted_root(answers: &Answers) -> Result<String, String> {
    if let Some(layout) = &answers.layout {
        let open = layout
            .opens
            .iter()
            .find(|open| open.target == "/")
            .ok_or_else(|| copy::AUTO_NO_ROOT.to_string())?;
        return Ok(open.partition.clone());
    }
    let listed = partitions(&answers.disk)?;
    let found = partitions_of_type(&listed, ROOT_GUID);
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(copy::AUTO_NO_ROOT.to_string()),
        many => Err(format!(
            "{} root partitions are on {}: {}",
            many.len(),
            answers.disk,
            many.join(", ")
        )),
    }
}

fn partitions(disk: &str) -> Result<String, String> {
    let out = Command::new("lsblk")
        .args(["-rno", "NAME,PARTTYPE"])
        .arg(disk)
        .output()
        .map_err(|err| format!("lsblk: {err}, and it is what names a partition"))?;
    if !out.status.success() {
        return Err(format!(
            "lsblk {disk}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub(crate) fn partitions_of_type(listed: &str, kind: &str) -> Vec<String> {
    listed
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let name = words.next()?;
            let found = words.next().unwrap_or("");
            found
                .eq_ignore_ascii_case(kind)
                .then(|| format!("/dev/{name}"))
        })
        .collect()
}

pub(crate) fn finalize(ready: &Ready) -> Result<(), String> {
    require_credential_tool(&ready.image)?;
    let before = slots(&ready.partition)?.keys;
    let slot = free_slot(&before)?;
    let key = random_key()?;
    let open = LuksOpen {
        partition: ready.partition.clone(),
        target: "/".to_string(),
        key: ready.key.clone(),
    };
    add_key(&open, &key, slot)?;
    let tested = test_key(&ready.partition, &key);
    if !matches!(tested, Ok(true)) {
        let why = match tested {
            Ok(_) => copy::added_key_wrong(&ready.partition),
            Err(why) => why,
        };
        return Err(discarded(ready, slot, why));
    }
    with_esp(&ready.disk, |esp| credential(&ready.image, esp, &key, slot))
        .map_err(|why| discarded(ready, slot, why))
}

fn require_credential_tool(image: &str) -> Result<(), String> {
    // PCR 11 measures the UKI.
    // Binding to PCR 7 would break when the firmware keys are enrolled.
    let out = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--net=none",
            "--security-opt",
            "label=disable",
            "--entrypoint",
            "",
        ])
        .arg(image)
        .args(["/usr/bin/test", "-x", "/usr/bin/systemd-creds"])
        .output()
        .map_err(|err| format!("podman: {err}, and it is what checks {image} for systemd-creds"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "{image} has no /usr/bin/systemd-creds, so the one-time key cannot be sealed for the first boot"
    ))
}

fn discarded(ready: &Ready, slot: u32, why: String) -> String {
    match kill_slot(ready, slot) {
        Ok(()) => why,
        Err(cleanup) => format!("{why}; and slot {slot} was left behind: {cleanup}"),
    }
}

fn kill_slot(ready: &Ready, slot: u32) -> Result<(), String> {
    let mut command = Command::new("cryptsetup");
    command.args(["-q", "luksKillSlot"]);
    command.args(auth_args(&ready.key));
    command.arg(&ready.partition).arg(slot.to_string());
    run_with_key(command, &ready.key, "removes a key")
        .map_err(|why| format!("removing slot {slot} from {}: {why}", ready.partition))
}

fn with_esp<T>(disk: &str, write: impl FnOnce(&Path) -> Result<T, String>) -> Result<T, String> {
    let listed = partitions(disk)?;
    let mut mounts = Mounts::default();
    for device in partitions_of_type(&listed, copy::ESP_TYPE) {
        let Ok(at) = mount_rw(&device, &mut mounts) else {
            continue;
        };
        if !at.join("loader/entries").is_dir() && !at.join("EFI/Linux").is_dir() {
            mounts.release(&at)?;
            continue;
        }
        let written = write(&at);
        let released = mounts.release(&at);
        return match (written, released) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(why), Ok(())) => Err(why),
            (Ok(_), Err(unmount)) => Err(unmount),
            (Err(why), Err(unmount)) => Err(format!("{why}; and {unmount}")),
        };
    }
    Err(format!(
        "{disk} carries no EFI system partition with the installed boot files"
    ))
}

fn credential(image: &str, esp: &Path, key: &[u8], slot: u32) -> Result<(), String> {
    let at = esp.join(CREDENTIAL_DIR);
    std::fs::create_dir_all(&at).map_err(|err| format!("{}: {err}", at.display()))?;
    let marker = at.join(SLOT_FILE);
    std::fs::write(&marker, format!("{slot}\n"))
        .map_err(|err| format!("{}: {err}", marker.display()))?;
    let staged = staged_key(key)?;
    let key_mount = format!("{}:/run/one-time-key:ro", staged.0.display());
    let esp_mount = format!("{}:/esp", esp.display());
    let out = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--net=none",
            "--security-opt",
            "label=disable",
        ])
        .arg("--device")
        .arg(tpm())
        .args(["--entrypoint", ""])
        .args(["-v", &key_mount, "-v", &esp_mount])
        .arg(image)
        .args(["/usr/bin/systemd-creds", "encrypt", "--with-key=tpm2"])
        .arg(format!("--name={CREDENTIAL}"))
        .arg(format!("--tpm2-public-key={PCR_POLICY}"))
        .args(["--tpm2-public-key-pcrs=11", "/run/one-time-key"])
        .arg(format!("/esp/{CREDENTIAL_DIR}/{CREDENTIAL}.cred"))
        .output()
        .map_err(|err| format!("podman: {err}, and it is what seals the one-time key"))?;
    if !out.status.success() {
        return Err(format!(
            "sealing the one-time key in {image}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}
