use super::*;

pub struct Encryption {
    pub kind: String,
    pub passphrase: String,
    pub pin: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CustomMount {
    pub(crate) partition: String,
    pub(crate) target: String,
    pub(crate) fstype: String,
    pub(crate) passphrase: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rename {
    pub(crate) partition: String,
    pub(crate) label: String,
}

#[derive(Clone, PartialEq)]
pub(crate) enum Key {
    Passphrase(String),
    File(PathBuf),
    Data(Vec<u8>),
}

impl Key {
    pub(crate) fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Passphrase(passphrase) => Some(passphrase.as_bytes()),
            Self::Data(bytes) => Some(bytes),
            Self::File(_) => None,
        }
    }
}

// A derived `Debug` would put the passphrase and the key bytes into a log or a panic message.
impl std::fmt::Debug for Key {
    fn fmt(&self, form: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Passphrase(_) => form.write_str("Passphrase(***)"),
            Self::Data(_) => form.write_str("Data(***)"),
            Self::File(path) => form.debug_tuple("File").field(path).finish(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LuksOpen {
    pub(crate) partition: String,
    pub(crate) target: String,
    pub(crate) key: Key,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mounted {
    pub(crate) target: String,
    pub(crate) fstype: String,
}

pub(crate) const OPEN: &str = "open";

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Created {
    pub(crate) gb: u64,
    pub(crate) offset: u64,
    pub(crate) target: String,
    pub(crate) fstype: String,
    pub(crate) label: String,
    pub(crate) device: String,
    pub(crate) encrypt: bool,
    pub(crate) discoverable: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CustomLayout {
    pub(crate) disk: String,
    pub(crate) mounts: Vec<CustomMount>,
    pub(crate) opens: Vec<LuksOpen>,
    pub(crate) deletes: Vec<String>,
    pub(crate) creates: Vec<Created>,
    pub(crate) renames: Vec<Rename>,
    pub(crate) esp: Vec<EspRemoval>,
    pub(crate) confirmed: Option<DiskState>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Partition {
    pub(crate) device: String,
    pub(crate) size: String,
    pub(crate) fstype: String,
    pub(crate) label: String,
    pub(crate) parttype: String,
    pub(crate) uuid: String,
}

impl CustomLayout {
    pub(crate) fn empty(disk: &str) -> Self {
        Self {
            disk: disk.to_string(),
            ..Default::default()
        }
    }

    pub(crate) fn answer(&self, partition: &Partition) -> Option<Mounted> {
        self.mounts
            .iter()
            .find(|mount| mount.partition == partition.device)
            .map(|mount| Mounted {
                target: mount.target.clone(),
                fstype: mount.fstype.clone(),
            })
            .or_else(|| {
                self.opens
                    .iter()
                    .find(|open| open.partition == partition.device)
                    .map(|open| Mounted {
                        target: open.target.clone(),
                        fstype: OPEN.to_string(),
                    })
            })
    }

    pub(crate) fn mappers(&self) -> Vec<(String, &LuksOpen)> {
        self.opens
            .iter()
            .enumerate()
            .map(|(at, open)| (format!("tect-{}", at + 1), open))
            .collect()
    }

    pub(crate) fn changes_table(&self) -> bool {
        !(self.deletes.is_empty() && self.creates.is_empty() && self.renames.is_empty())
    }

    pub(crate) fn opens_the_root(&self) -> bool {
        self.opens.iter().any(|open| open.target == "/")
    }

    pub(crate) fn renamed(&self, partition: &str) -> Option<&str> {
        self.renames
            .iter()
            .find(|rename| rename.partition == partition)
            .map(|rename| rename.label.as_str())
    }

    pub(crate) fn formatted(&self, partition: &str) -> bool {
        self.mounts
            .iter()
            .any(|mount| mount.partition == partition && mount.fstype != "unformatted")
    }

    pub(crate) fn entry_replaced(&self, link: &str) -> bool {
        !link.is_empty()
            && (self.deletes.iter().any(|at| at == link)
                || self.mounts.iter().any(|mount| mount.partition == link)
                || self.opens.iter().any(|open| open.partition == link))
    }

    pub(crate) fn entry_gone(&self, link: &str) -> bool {
        !link.is_empty() && (self.deletes.iter().any(|at| at == link) || self.formatted(link))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Slots {
    pub(crate) keys: Vec<u32>,
    pub(crate) tokens: Vec<String>,
}

impl Slots {
    pub(crate) fn has_tpm2(&self) -> bool {
        self.tokens.iter().any(|kind| kind == "systemd-tpm2")
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Opened {
    Keep,
    Tpm2,
    AddKey,
}

impl Opened {
    pub(crate) fn of(shown: &str) -> Self {
        match shown {
            copy::OPENED_TPM2 => Self::Tpm2,
            copy::OPENED_ADD_KEY => Self::AddKey,
            _ => Self::Keep,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Keep => copy::OPENED_KEEP,
            Self::Tpm2 => copy::OPENED_TPM2,
            Self::AddKey => copy::OPENED_ADD_KEY,
        }
    }
}

pub(crate) fn at_boot(open: &LuksOpen, opened: Opened) -> &'static str {
    if opened == Opened::Tpm2 {
        return copy::BOOT_TPM2;
    }
    if open.target == "/" {
        return match open.key {
            Key::Passphrase(_) => copy::BOOT_PASSPHRASE,
            _ => copy::OPENED_ROOT_KEYFILE,
        };
    }
    match (&open.key, opened) {
        (Key::Passphrase(_), Opened::AddKey) => copy::BOOT_ADDED_KEY,
        (Key::Passphrase(_), _) => copy::BOOT_PASSPHRASE,
        _ => copy::BOOT_KEYFILE,
    }
}

impl Encryption {
    pub(crate) fn wants_passphrase(kind: &str) -> bool {
        kind.ends_with("passphrase")
    }

    pub(crate) fn wants_pin(kind: &str) -> bool {
        kind.ends_with("pin")
    }
}

pub struct Answers {
    pub disk: String,
    pub hostname: String,
    pub user: String,
    pub password: String,
    pub encryption: Encryption,
    pub(crate) layout: Option<CustomLayout>,
    pub(crate) opened: Opened,
}

#[derive(Clone, Default)]
pub struct Given {
    pub disk: Option<String>,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub encryption: Option<String>,
    pub passphrase: Option<String>,
    pub pin: Option<String>,
}

pub(crate) enum Leave {
    Back,
    Over,
    Shell,
}

pub(crate) fn leaving(err: &str) -> bool {
    err == common::ui::INTERRUPTED || common::prompt::cancelled(err)
}

pub(crate) fn leave_options() -> [Choice; 3] {
    [
        Choice::new(copy::LEAVE_BACK, ""),
        Choice::new(copy::LEAVE_OVER, ""),
        Choice::new(copy::LEAVE_SHELL, ""),
    ]
}

pub(crate) fn leave(prompt: &Prompt) -> Result<Leave, String> {
    if !prompt.draws() {
        return Ok(Leave::Shell);
    }
    let options = leave_options();
    match prompt.choose(copy::LEAVING, &options) {
        Ok(Some(1)) => Ok(Leave::Over),
        Ok(Some(2)) => Ok(Leave::Shell),
        Ok(_) => Ok(Leave::Back),
        Err(err) if leaving(&err) => Ok(Leave::Back),
        Err(err) => Err(err),
    }
}
