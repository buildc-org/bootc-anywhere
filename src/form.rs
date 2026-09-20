use super::*;

pub(crate) const ROW_HOSTNAME: usize = 0;
pub(crate) const ROW_ACCOUNT: usize = 1;
pub(crate) const ROW_PASSWORD: usize = 2;
pub(crate) const ROW_CONFIRM: usize = 3;
pub(crate) const ROW_LAYOUT: usize = 4;
pub(crate) const ROW_ENCRYPTION: usize = 5;
pub(crate) const ROW_DATA: usize = 6;
pub(crate) const ROW_SIZE: usize = 7;
pub(crate) const ROW_TABLE: usize = 8;
pub(crate) const ROW_PASSPHRASE: usize = 9;
pub(crate) const ROW_PIN: usize = 10;

pub(crate) fn held_pick(
    fields: &[common::ui::Field],
    layout: Option<&CustomLayout>,
    disk: &str,
) -> Option<CustomLayout> {
    match fields[ROW_LAYOUT].value() == copy::LAYOUT_MANUAL {
        true => Some(match layout {
            Some(held) if held.disk == disk => held.clone(),
            _ => CustomLayout::empty(disk),
        }),
        false => None,
    }
}

pub(crate) fn asked(fields: &[common::ui::Field]) -> Vec<usize> {
    let manual = fields[ROW_LAYOUT].value() == copy::LAYOUT_MANUAL;
    let opens = matches!(fields[ROW_ENCRYPTION], common::ui::Field::Pick { .. });
    let sized = fields[ROW_DATA].value() == copy::DATA_SEPARATE;
    (0..fields.len())
        .filter(|row| *row != ROW_PASSPHRASE && *row != ROW_PIN)
        .filter(|row| {
            !manual
                || (*row != ROW_ENCRYPTION && *row != ROW_DATA && *row != ROW_SIZE)
                || (opens && *row == ROW_ENCRYPTION)
        })
        .filter(|row| manual || *row != ROW_SIZE || sized)
        .collect()
}

pub(crate) fn pair_mismatch(fields: &[common::ui::Field], pair: [usize; 2]) -> Vec<usize> {
    let (left, right) = (fields[pair[0]].value(), fields[pair[1]].value());
    match left.is_empty() || right.is_empty() || left == right {
        true => Vec::new(),
        false => pair.to_vec(),
    }
}

pub(crate) fn short_of(
    fields: &[common::ui::Field],
    disk: &str,
    layout: Option<&CustomLayout>,
    payload: &Payload,
    tpm: bool,
    table: Option<&DiskTable>,
) -> Option<String> {
    let at = |row: usize| fields[row].value();
    let held = held_pick(fields, layout, disk);
    let manual = held.is_some();
    if !pair_mismatch(fields, [ROW_PASSWORD, ROW_CONFIRM]).is_empty() {
        // An empty refusal blocks the action without drawing a reason, because the mismatched pair is already marked.
        return Some(String::new());
    }
    if let Some(layout) = held.as_ref() {
        if let Some(short) = layout_short_of(layout, payload.composefs, table, payload.reserve_gb())
        {
            return Some(short);
        }
        if layout.opens_the_root() && !payload.luks_initramfs {
            return Some(copy::NO_LUKS_INITRAMFS.to_string());
        }
        if layout
            .opens
            .iter()
            .any(|open| open.target == "/" && !matches!(open.key, Key::Passphrase(_)))
        {
            return Some(copy::OPENED_ROOT_KEYFILE.to_string());
        }
        if !layout.opens_the_root() && Opened::of(&at(ROW_ENCRYPTION)) != Opened::Keep {
            return Some(copy::OPENED_KEYFILE_PLAIN.to_string());
        }
        if !layout.opens_the_root()
            && layout
                .opens
                .iter()
                .any(|open| open.target != "/" && !matches!(open.key, Key::Passphrase(_)))
        {
            return Some(copy::OPENED_KEYFILE_PLAIN.to_string());
        }
    }
    let kind = match manual {
        true => NONE,
        false => written(&at(ROW_ENCRYPTION)),
    };
    if kind != NONE && !payload.luks_initramfs {
        return Some(copy::NO_LUKS_INITRAMFS.to_string());
    }
    if kind.starts_with("tpm2") && !tpm {
        return Some(copy::NO_TPM.to_string());
    }
    if !manual && at(ROW_DATA) == copy::DATA_SEPARATE && !at(ROW_SIZE).is_empty() {
        if let (Some((disk, disk_gb)), Some(home)) = (chosen_disk(fields), size_gb(&at(ROW_SIZE))) {
            if home < 1 {
                return Some(copy::home_too_small(&copy::size_said(&at(ROW_SIZE))));
            }
            let boot = u64::from(payload.bootloader != "systemd") * 2;
            let room = disk_gb.saturating_sub(2 + boot + payload.reserve_gb());
            if home > room {
                return Some(copy::home_too_big(&copy::size_said(&at(ROW_SIZE)), &disk));
            }
        }
    }
    let wants = Encryption::wants_passphrase(kind);
    let wants_pin = Encryption::wants_pin(kind);
    let missing: Vec<&str> = [
        (at(ROW_HOSTNAME).is_empty(), copy::ROW_HOSTNAME),
        (disk.is_empty(), copy::INSTALLATION_DISK),
        (at(ROW_ACCOUNT).is_empty(), copy::ROW_ACCOUNT),
        (at(ROW_PASSWORD).is_empty(), copy::ROW_PASSWORD),
        (
            !at(ROW_PASSWORD).is_empty() && at(ROW_CONFIRM).is_empty(),
            copy::ROW_CONFIRM,
        ),
        (wants && at(ROW_PASSPHRASE).is_empty(), copy::ROW_PASSPHRASE),
        (wants_pin && at(ROW_PIN).is_empty(), copy::ROW_PIN),
        (
            !manual && at(ROW_DATA) == copy::DATA_SEPARATE && at(ROW_SIZE).is_empty(),
            copy::ROW_SIZE,
        ),
    ]
    .into_iter()
    .filter(|(missing, _)| *missing)
    .map(|(_, name)| name)
    .collect();
    match missing.is_empty() {
        true => None,
        false => Some(copy::still_needs(&missing)),
    }
}

fn chosen_disk(fields: &[common::ui::Field]) -> Option<(String, u64)> {
    let common::ui::Field::Table { rows, .. } = &fields[ROW_TABLE] else {
        return None;
    };
    let row = rows
        .iter()
        .find(|row| row.first().is_some_and(|cell| cell.answered()))?;
    let size = row.get(1)?;
    Some((size.text().to_string(), size_gb(size.text())?))
}
