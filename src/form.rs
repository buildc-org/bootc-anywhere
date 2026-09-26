use super::*;

pub(crate) const ROW_HOSTNAME: usize = 0;
pub(crate) const ROW_ACCOUNT: usize = 1;
pub(crate) const ROW_PASSWORD: usize = 2;
pub(crate) const ROW_CONFIRM: usize = 3;
pub(crate) const ROW_LAYOUT: usize = 4;
pub(crate) const ROW_ENCRYPTION: usize = 5;
pub(crate) const ROW_TABLE: usize = 6;
pub(crate) const ROW_PASSPHRASE: usize = 7;
pub(crate) const ROW_PIN: usize = 8;

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
    (0..fields.len())
        .filter(|row| *row != ROW_PASSPHRASE && *row != ROW_PIN)
        .filter(|row| !manual || *row != ROW_ENCRYPTION || opens)
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
    let account = at(ROW_ACCOUNT);
    if !account.is_empty() && !portable_name(&account) {
        return Some(copy::account_name(&account));
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
