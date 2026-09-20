use std::path::Path;

const SECURE_BOOT: &str = "SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";
const SETUP_MODE: &str = "SetupMode-8be4df61-93ca-11d2-aa0d-00e098032b8c";
const PLATFORM_KEY: &str = "PK-8be4df61-93ca-11d2-aa0d-00e098032b8c";
const DATABASE: &str = "db-d719b2cb-3d3a-4596-a3bc-dad00e67656f";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKey {
    None,
    Other,
    Owner,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Firmware {
    pub efivars: bool,
    pub secure_boot: Option<bool>,
    pub setup_mode: Option<bool>,
    pub platform_key: PlatformKey,
    pub database: bool,
}

impl Default for Firmware {
    fn default() -> Self {
        Firmware {
            efivars: false,
            secure_boot: None,
            setup_mode: None,
            platform_key: PlatformKey::None,
            database: false,
        }
    }
}

impl Firmware {
    pub fn read(root: &Path, owner: Option<&[u8]>) -> Firmware {
        let Ok(entries) = std::fs::read_dir(root) else {
            return Firmware::default();
        };
        let mut firmware = Firmware {
            efivars: true,
            ..Firmware::default()
        };
        let mut read_key = None;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !matches!(
                name.as_ref(),
                SECURE_BOOT | SETUP_MODE | PLATFORM_KEY | DATABASE
            ) {
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            // The UEFI variable attributes occupy the first four bytes, before the payload.
            let value = bytes.get(4..).unwrap_or_default();
            match name.as_ref() {
                SECURE_BOOT => firmware.secure_boot = value.first().map(|byte| *byte == 1),
                SETUP_MODE => firmware.setup_mode = value.first().map(|byte| *byte == 1),
                PLATFORM_KEY => read_key = Some(key_of(value, owner)),
                DATABASE => {
                    firmware.database = owner.is_some_and(|owner| holds(value, owner));
                }
                _ => {}
            }
        }
        firmware.platform_key = match read_key {
            Some(key) => key,
            None => match firmware.setup_mode {
                Some(true) => PlatformKey::None,
                _ => PlatformKey::Unknown,
            },
        };
        firmware
    }
}

fn key_of(value: &[u8], owner: Option<&[u8]>) -> PlatformKey {
    match value.is_empty() {
        true => PlatformKey::None,
        false => match owner {
            Some(owner) if holds(value, owner) => PlatformKey::Owner,
            _ => PlatformKey::Other,
        },
    }
}

fn holds(value: &[u8], owner: &[u8]) -> bool {
    !owner.is_empty() && value.windows(owner.len()).any(|window| window == owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variable(root: &Path, name: &str, value: &[u8]) {
        std::fs::create_dir_all(root).unwrap();
        let file = match name {
            "db" => format!("{name}-d719b2cb-3d3a-4596-a3bc-dad00e67656f"),
            _ => format!("{name}-8be4df61-93ca-11d2-aa0d-00e098032b8c"),
        };
        let mut bytes = vec![0, 0, 0, 7];
        bytes.extend_from_slice(value);
        std::fs::write(root.join(file), bytes).unwrap();
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("tect-firmware-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn read(name: &str, owner: Option<&[u8]>, variables: &[(&str, &[u8])]) -> Firmware {
        let root = fixture(name);
        for (name, value) in variables {
            variable(&root, name, value);
        }
        let firmware = Firmware::read(&root, owner);
        let _ = std::fs::remove_dir_all(&root);
        firmware
    }

    #[test]
    fn a_machine_without_efivars_is_neither_keyless_nor_enrolled() {
        let firmware = Firmware::read(&fixture("absent"), None);
        assert!(!firmware.efivars, "{firmware:?}");
        assert_eq!(firmware.secure_boot, None, "{firmware:?}");
        assert_eq!(firmware.platform_key, PlatformKey::None, "{firmware:?}");
    }

    #[test]
    fn the_variables_are_read_apart_from_the_owner() {
        let firmware = read(
            "present",
            Some(b"owner certificate"),
            &[
                ("SecureBoot", &[1]),
                ("SetupMode", &[0]),
                ("PK", b"vendor certificate bytes"),
                ("db", b"vendor database bytes"),
            ],
        );
        assert!(firmware.efivars && firmware.secure_boot == Some(true));
        assert_eq!(firmware.setup_mode, Some(false));
        assert_eq!(firmware.platform_key, PlatformKey::Other);
        assert!(!firmware.database, "{firmware:?}");
    }

    #[test]
    fn an_enrolled_owner_certificate_is_recognised_in_place() {
        let firmware = read(
            "owner",
            Some(b"owner certificate"),
            &[
                ("SecureBoot", &[1]),
                ("SetupMode", &[0]),
                ("PK", b"list-header owner certificate list-footer"),
                ("db", b"list-header owner certificate list-footer"),
            ],
        );
        assert_eq!(firmware.platform_key, PlatformKey::Owner, "{firmware:?}");
        assert!(firmware.database, "{firmware:?}");

        assert_eq!(key_of(b"", Some(b"owner certificate")), PlatformKey::None);
        assert_eq!(
            key_of(b"another", Some(b"owner certificate")),
            PlatformKey::Other
        );
        assert!(
            !holds(b"owner cer", b"owner certificate"),
            "a value shorter than the certificate cannot hold it"
        );
        assert!(!holds(b"another certificate", b"owner certificate"));
        assert!(!holds(b"another", b""));
    }

    #[test]
    fn a_key_the_firmware_claims_but_this_cannot_read_is_unknown() {
        let firmware = read(
            "unread-pk",
            None,
            &[("SecureBoot", &[1]), ("SetupMode", &[0])],
        );
        assert_eq!(firmware.platform_key, PlatformKey::Unknown, "{firmware:?}");
        let setup = read("setup", None, &[("SecureBoot", &[0]), ("SetupMode", &[1])]);
        assert_eq!(setup.platform_key, PlatformKey::None, "{setup:?}");
        assert_eq!(setup.setup_mode, Some(true), "{setup:?}");
        let bare = read("bare", None, &[("SecureBoot", &[0])]);
        assert_eq!(bare.platform_key, PlatformKey::Unknown, "{bare:?}");
        assert_eq!(bare.setup_mode, None, "{bare:?}");
    }
}
