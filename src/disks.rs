use super::*;

// The kernel reports `/sys/block/<disk>/size` in 512-byte sectors, whatever the disk's logical sector size.
const SECTOR: u64 = 512;

const VIRTUAL: [&str; 7] = ["loop", "ram", "zram", "sr", "fd", "dm-", "md"];

pub(crate) fn in_use_now() -> String {
    std::fs::read_to_string("/proc/mounts").unwrap_or_default()
}

fn in_use(disk: &Path, name: &str, mounts: &str) -> bool {
    let is_source = |dev: &str| {
        mounts
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .any(|source| source == dev)
    };
    if is_source(&format!("/dev/{name}")) {
        return true;
    }
    let Ok(parts) = std::fs::read_dir(disk) else {
        return false;
    };
    parts.flatten().any(|part| {
        part.path().join("partition").is_file()
            && is_source(&format!("/dev/{}", part.file_name().to_string_lossy()))
    })
}

pub fn disks(sys: &Path, mounts: &str) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(sys) else {
        return Vec::new();
    };
    let mut found: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if VIRTUAL.iter().any(|prefix| name.starts_with(prefix)) {
            continue;
        }
        let read = |leaf: &str| {
            std::fs::read_to_string(entry.path().join(leaf))
                .map(|text| text.trim().to_string())
                .unwrap_or_default()
        };
        let sectors: u64 = read("size").parse().unwrap_or(0);
        if sectors == 0 {
            continue;
        }
        if read("ro") == "1" || in_use(&entry.path(), &name, mounts) {
            continue;
        }
        let removable = match read("removable").as_str() {
            "1" => copy::REMOVABLE.to_string(),
            _ => String::new(),
        };
        let gigs = sectors as f64 * SECTOR as f64 / 1_000_000_000.0;
        let detail = [
            copy::size_said(&format!("{gigs:.1}")),
            read("device/model"),
            removable,
        ];
        found.push((
            format!("/dev/{name}"),
            detail
                .iter()
                .filter(|part| !part.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join("  "),
        ));
    }
    found.sort();
    found
}

pub(crate) fn ask_disk(
    given: Option<String>,
    current: Option<&str>,
    prompt: &Prompt,
) -> Result<String, String> {
    if let Some(disk) = given.filter(|disk| !disk.is_empty()) {
        return Ok(disk);
    }
    let found = disks(&sys_block(), &in_use_now());
    if !prompt.asks() || found.is_empty() {
        return prompt.text(None, copy::INSTALL_DISK, "--disk", current);
    }
    let options: Vec<Choice> = found
        .iter()
        .map(|(disk, detail)| Choice::new(disk, detail))
        .collect();
    let at = current
        .and_then(|held| found.iter().position(|(disk, _)| disk == held))
        .unwrap_or(0);
    match prompt.choose_current(copy::INSTALL_DISK, &options, at)? {
        Some(at) => Ok(found[at].0.clone()),
        None => Ok(current.unwrap_or_default().to_string()),
    }
}

pub(crate) fn named(kind: String) -> Result<String, String> {
    if KINDS.iter().any(|(name, _, _)| *name == kind) {
        return Ok(kind);
    }
    Err(format!(
        "`{kind}` is not one of {}",
        KINDS
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

pub(crate) fn written(shown: &str) -> &'static str {
    KINDS
        .iter()
        .find(|(_, label, _)| *label == shown)
        .map_or(NONE, |(name, _, _)| *name)
}

pub(crate) fn shown(kind: &str) -> &'static str {
    KINDS
        .iter()
        .find(|(name, _, _)| *name == kind)
        .map_or(copy::ENC_NONE, |(_, label, _)| *label)
}

pub(crate) fn kind_at(options: &[Choice], kind: &str) -> Option<usize> {
    options
        .iter()
        .position(|choice| choice.label == shown(kind))
}

pub(crate) fn kinds(tpm: bool, luks_initramfs: bool, with_none: bool) -> Vec<Choice> {
    KINDS
        .iter()
        .filter(|(name, _, _)| with_none || *name != NONE)
        .map(|(name, shown, detail)| match () {
            _ if *name != NONE && !luks_initramfs => Choice::new(*shown, copy::NO_LUKS_INITRAMFS)
                .unavailable()
                .hidden(),
            _ if !tpm && name.starts_with("tpm2") => {
                Choice::new(*shown, copy::NO_TPM).unavailable().hidden()
            }
            _ => Choice::new(*shown, *detail),
        })
        .collect()
}

pub(crate) fn visible_kinds(tpm: bool, luks_initramfs: bool) -> Vec<(usize, Choice)> {
    kinds(tpm, luks_initramfs, true)
        .into_iter()
        .enumerate()
        .filter(|(_, choice)| !choice.hidden)
        .collect()
}

pub(crate) fn encryption_row(
    layout: Option<&CustomLayout>,
    held: &common::ui::Field,
    tpm: bool,
    luks_initramfs: bool,
) -> common::ui::Field {
    match layout.filter(|layout| !layout.opens.is_empty()) {
        Some(layout) => {
            let chosen = Opened::of(&held.value());
            let rows = opened_rows(layout, tpm, luks_initramfs);
            let at = rows
                .iter()
                .position(|row| row.label == chosen.label())
                .or_else(|| rows.iter().position(|row| row.label == copy::OPENED_KEEP));
            common::ui::Field::pick(copy::ROW_ENCRYPTION, rows, at)
        }
        None => common::ui::Field::action(copy::ROW_ENCRYPTION, shown(written(&held.value()))),
    }
}

pub(crate) fn opened_rows(layout: &CustomLayout, tpm: bool, luks_initramfs: bool) -> Vec<Choice> {
    let read: Vec<Result<Slots, String>> = layout
        .opens
        .iter()
        .map(|open| slots(&open.partition))
        .collect();
    let said = match read.is_empty() {
        true => String::new(),
        false => read
            .iter()
            .map(|slots| match slots {
                Ok(slots) => copy::slots_said(&slots.keys, &slots.tokens),
                Err(why) => copy::slots_unknown(why),
            })
            .collect::<Vec<_>>()
            .join("; "),
    };
    let mut rows = vec![Choice::new(copy::OPENED_KEEP, said)];
    let keyfile_root = layout
        .opens
        .iter()
        .any(|open| open.target == "/" && !matches!(open.key, Key::Passphrase(_)));
    if keyfile_root {
        rows[0] = Choice::new(copy::OPENED_KEEP, copy::OPENED_ROOT_KEYFILE)
            .unavailable()
            .hidden();
    }
    let unread = read.iter().find_map(|slots| slots.as_ref().err());
    let enrolled = read
        .iter()
        .any(|slots| slots.as_ref().is_ok_and(Slots::has_tpm2));
    let encrypted_root = layout.opens_the_root();
    rows.push(match (tpm, unread, enrolled) {
        (false, _, _) => Choice::new(copy::OPENED_TPM2, copy::NO_TPM)
            .unavailable()
            .hidden(),
        _ if !encrypted_root => Choice::new(copy::OPENED_TPM2, copy::OPENED_KEYFILE_PLAIN)
            .unavailable()
            .hidden(),
        (_, Some(why), _) => Choice::new(copy::OPENED_TPM2, why.clone())
            .unavailable()
            .hidden(),
        (_, _, true) => Choice::new(copy::OPENED_TPM2, copy::OPENED_TPM2_HAS)
            .unavailable()
            .hidden(),
        _ if keyfile_root => Choice::new(copy::OPENED_TPM2, copy::OPENED_ROOT_KEYFILE)
            .unavailable()
            .hidden(),
        _ => Choice::new(copy::OPENED_TPM2, copy::OPENED_TPM2_COST),
    });
    if let Some((_, slots)) = layout
        .opens
        .iter()
        .zip(read.iter())
        .find(|(open, _)| open.target != "/" && matches!(open.key, Key::Passphrase(_)))
    {
        let detail = match slots {
            Ok(slots) => format!(
                "{}; {} slots now",
                copy::OPENED_ADD_KEY_COST,
                slots.keys.len()
            ),
            Err(_) => copy::OPENED_ADD_KEY_COST.to_string(),
        };
        rows.push(match encrypted_root {
            true => Choice::new(copy::OPENED_ADD_KEY, detail),
            false => Choice::new(copy::OPENED_ADD_KEY, copy::OPENED_KEYFILE_PLAIN)
                .unavailable()
                .hidden(),
        });
    }
    if layout.opens_the_root() && !luks_initramfs {
        for row in &mut rows {
            *row = Choice::new(row.label.clone(), copy::NO_LUKS_INITRAMFS)
                .unavailable()
                .hidden();
        }
    }
    rows
}

pub(crate) fn slots(container: &str) -> Result<Slots, String> {
    let out = Command::new("cryptsetup")
        .args(["luksDump", "--dump-json-metadata"])
        .arg(container)
        .output()
        .map_err(|err| format!("cryptsetup: {err}, and it is what reads a header"))?;
    match out.status.success() {
        true => slots_from(&String::from_utf8_lossy(&out.stdout)),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

pub(crate) fn slots_from(raw: &str) -> Result<Slots, String> {
    const SLOTS: u32 = 32;
    let doc = Json::parse(raw).map_err(|err| format!("luksDump wrote invalid JSON: {err}"))?;
    let mut keys = Vec::new();
    if let Some(keyslots) = json::field(&doc, "keyslots") {
        for at in 0..SLOTS {
            if json::field(keyslots, &at.to_string()).is_some() {
                keys.push(at);
            }
        }
    }
    let mut tokens = Vec::new();
    if let Some(listed) = json::field(&doc, "tokens") {
        let Json::Object(entries) = listed else {
            return Err("luksDump wrote tokens that are not an object".to_string());
        };
        for (_, token) in entries {
            if let Some(kind) = json::text(token, "type") {
                tokens.push(kind);
            }
        }
    }
    tokens.sort();
    Ok(Slots { keys, tokens })
}

pub(crate) fn data_rows(composefs: bool) -> Vec<Choice> {
    let separate = Choice::new(copy::DATA_SEPARATE, "");
    vec![
        Choice::new(copy::DATA_TOGETHER, ""),
        match composefs {
            true => separate.hidden(),
            false => separate,
        },
    ]
}

pub(crate) fn chose(shown: &str, size: &str) -> Data {
    match shown == copy::DATA_SEPARATE {
        true => Data {
            size: size.to_string(),
        },
        false => Data::default(),
    }
}

pub(crate) fn data_said(data: &Data) -> String {
    match data.size.is_empty() {
        true => copy::DATA_TOGETHER.to_string(),
        false => copy::DATA_SEPARATE.to_string(),
    }
}

pub(crate) fn ask_encryption(
    given: Option<String>,
    passphrase: Option<String>,
    pin: Option<String>,
    prompt: &Prompt,
    tpm: bool,
    luks_initramfs: bool,
) -> Result<Encryption, String> {
    let kind = match given {
        Some(kind) => named(kind)?,
        None if !prompt.asks() => NONE.to_string(),
        None => {
            let options = visible_kinds(tpm, luks_initramfs);
            let shown: Vec<Choice> = options.iter().map(|(_, choice)| choice.clone()).collect();
            match prompt.choose_current(copy::INSTALL_ENCRYPTION, &shown, 0)? {
                Some(at) => KINDS[options[at].0].0.to_string(),
                None => NONE.to_string(),
            }
        }
    };
    if kind.starts_with("tpm2") && !tpm {
        return Err(copy::NO_TPM.to_string());
    }
    if kind != NONE && !luks_initramfs {
        return Err(copy::NO_LUKS_INITRAMFS.to_string());
    }
    Ok(Encryption {
        passphrase: match Encryption::wants_passphrase(&kind) {
            true => prompt.text(passphrase, copy::LUKS_PASSPHRASE, "--passphrase", None)?,
            false => String::new(),
        },
        pin: match Encryption::wants_pin(&kind) {
            true => prompt.text(pin, copy::LUKS_PIN, "--pin", None)?,
            false => String::new(),
        },
        kind,
    })
}
