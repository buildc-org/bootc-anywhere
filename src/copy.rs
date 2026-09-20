pub const INSTALL_DISK: &str = "Installation disk";
pub const INSTALL_NAME: &str = "Computer name";
pub const INSTALL_USER: &str = "Username";
pub const INSTALL_PASSWORD: &str = "Password";
pub const INSTALL_ENCRYPTION: &str = "Encryption type";
pub const LUKS_PASSPHRASE: &str = "LUKS passphrase";
pub const LUKS_PIN: &str = "LUKS PIN";
pub const ENC_NONE: &str = "none";
pub const ENC_TPM2: &str = "LUKS with TPM2 + recovery key";
pub const ENC_PASSPHRASE: &str = "LUKS with passphrase";
pub const ENC_BOTH: &str = "LUKS with TPM2 + recovery passphrase";
pub const ENC_TPM2_PIN: &str = "LUKS with TPM2 + PIN";
pub const NO_TPM: &str = "No TPM available";
pub const NO_LUKS_INITRAMFS: &str = "the image has not declared and proved `luks-initramfs`";
pub const REMOVABLE: &str = "removable";

pub const DATA_TOGETHER: &str = "System and Home on same partition";
pub const DATA_SEPARATE: &str = "Separate Home partition";
pub const DATA_ENCRYPTED: &str = "encrypted with the root's passphrase";
pub const CUSTOM_ROOT: &str = "still needs a / partition";
pub const CUSTOM_ESP: &str = "still needs a /boot/efi partition";
pub const CUSTOM_LUKS_LATER: &str = "manual LUKS is not installed yet; use the whole disk";
pub const CUSTOM_VERITY: &str = "the image needs fs-verity: format the root ext4 or btrfs";
pub const INSTALLATION_DISK: &str = "installation disk";
pub const CUSTOM_DUPLICATE: &str = "each mount point can be used only once";
pub const CUSTOM_UNMOUNTED: &str = "a partition is answered with no mount point";
pub const CUSTOM_UNFORMATTED: &str = "a new partition has no filesystem chosen";
pub fn table_already_changed(disk: &str) -> String {
    format!("the partition table on {disk} has already been changed")
}

pub fn table_changed(disk: &str) -> String {
    format!("the partition table on {disk} changed while it was being reviewed; review it again")
}

pub fn custom_not_gpt(label: &str) -> String {
    format!("this disk has a {label} partition table: clear it to use GPT")
}
pub fn custom_too_big(gb: u64) -> String {
    match gb {
        0 => "no room left on the disk for a new partition".to_string(),
        gb => format!("only {gb} GB is free after the last partition"),
    }
}

pub fn opened(partition: &str, how: &str) -> String {
    format!("{partition}  {how}")
}

pub const OPENED_KEEP: &str = "keep what is enrolled";
pub const OPENED_TPM2: &str = "add a TPM2 token";
pub const OPENED_ADD_KEY: &str = "add a key file for boot";
pub const OPENED_TPM2_COST: &str = "first boot asks once, then no prompt";
pub const OPENED_TPM2_HAS: &str = "this container already has one";
pub const OPENED_ADD_KEY_COST: &str = "this machine reads it, the old one keeps its own";
pub const OPENED_KEYFILE_PLAIN: &str = "the root is not encrypted, so the key would be readable";
pub const OPENED_ROOT_KEYFILE: &str = "a key file cannot open the root; use its passphrase";

pub fn slots_said(keys: &[u32], tokens: &[String]) -> String {
    let said: Vec<String> = keys
        .iter()
        .map(|at| format!("slot {at}"))
        .chain(tokens.iter().map(|kind| format!("token {kind}")))
        .collect();
    match said.is_empty() {
        true => "no slots".to_string(),
        false => said.join(", "),
    }
}

pub fn slots_unknown(why: &str) -> String {
    format!("its slots could not be read: {why}")
}

pub const BOOT_KEYFILE: &str = "decrypted, keyfile at boot";
pub const BOOT_PASSPHRASE: &str = "decrypted, passphrase at boot";
pub const BOOT_TPM2: &str = "decrypted, TPM2 at boot";
pub const BOOT_ADDED_KEY: &str = "decrypted, key added here";

pub fn kill_slot(device: &str, slot: u32, count: usize) -> Vec<String> {
    vec![
        format!("{device} has {count} slots, and this machine no longer needs slot {slot}"),
        format!("both keys open it, so it can go: cryptsetup luksKillSlot {device} {slot}"),
    ]
}

pub fn kill_a_slot(device: &str, count: usize) -> Vec<String> {
    vec![
        format!("{device} has {count} slots, and this machine no longer needs one of them"),
        format!("cryptsetup luksKillSlot {device} <slot>"),
    ]
}

pub fn crypttab_line(name: &str, uuid: &str, keyfile: Option<&str>) -> String {
    format!("{name} UUID={uuid} {} luks", keyfile.unwrap_or("none"))
}

pub const ENROLLING: &str = "enrolling disk auto-unlock; this takes a few seconds";

pub fn tpm2_unit(name: &str, key: &str, uuid: &str, pcr_policy: bool) -> String {
    let enroll = match pcr_policy {
        true => {
            "--tpm2-pcrs=7 --tpm2-public-key=/run/systemd/tpm2-pcr-public-key.pem \
                 --tpm2-signature=/run/systemd/tpm2-pcr-signature.json"
        }
        false => "--tpm2-pcrs=7",
    };
    format!(
        "[Unit]\n\
         Description=Enroll the {name} container for TPM2 unlock\n\
         ConditionPathExists={key}\n\
         After=basic.target\n\
         Before=systemd-user-sessions.service\n\
         DefaultDependencies=no\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         RemainAfterExit=no\n\
         TimeoutStartSec=120\n\
         StandardOutput=journal+console\n\
         StandardError=journal+console\n\
         ExecStartPre=/bin/echo '{ENROLLING}'\n\
         ExecStart=/usr/bin/systemd-cryptenroll --tpm2-device=auto {enroll} \
         --unlock-key-file={key} /dev/disk/by-uuid/{uuid}\n\
         ExecStartPost=-/usr/bin/shred -u {key}\n\
         ExecStartPost=-/usr/bin/systemctl disable tect-tpm2-enroll-{name}.service\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    )
}

pub fn added_key_wrong(device: &str) -> String {
    format!("the key just added to {device} does not open it")
}

pub const KEY_TYPE: &str = "Key type";
pub const KEY_OPEN: &str = "Open";
pub const KEY_BACK: &str = "Back";
pub const PARTITION_ENCRYPTION: &str = "partition encryption";
pub const CONFIRM_PASSPHRASE: &str = "confirm passphrase";
pub const CONFIRM_PIN: &str = "confirm PIN";
pub const LUKS_PARTITION: &str = "luks partition";
pub const ENCRYPTION_TYPE: &str = "encryption type";
pub const SUBMIT_KEYS: &str = "\u{23ce}  submit \u{2022} Esc back \u{2022} Alt+S show";
pub const KEY_PASSPHRASE: &str = "passphrase";
pub const KEY_FILE: &str = "key file";
pub const KEY_FILE_PATH: &str = "key file path";
pub const KEY_FILE_MISSING: &str = "there is no file at that path";
pub const ENCRYPTED_DISK: &str = "encrypted disk";
pub const ENCRYPTED_PARTITION: &str = "encrypted partition";

pub const OLD_ROOT_NONE: &str = "no partition on this disk reads as an old system";
pub fn old_root_unnamed(partition: &str) -> String {
    format!("no old system on this disk names a key for {partition}")
}
pub fn old_root_partly(unread: &str) -> String {
    format!("{unread}, and nothing readable named a key")
}
pub fn old_root_key_missing(volume: &str, at: &str, why: &str) -> String {
    format!("the key {at} that opens {volume} could not be read: {why}")
}
pub fn old_root_key_outside(at: &str) -> String {
    format!("the key {at} is not a file inside the system that names it")
}
pub fn old_root_key_wrong(partition: &str) -> String {
    format!("the key the old system named for {partition} does not open it")
}
pub fn old_root_key_unreadable(volume: &str) -> String {
    format!("{volume} opens with a script this installer cannot run")
}
pub fn old_root_key_removable(volume: &str) -> String {
    format!("{volume} opens with a key file on removable media, which this installer does not copy onto the machine")
}

pub fn old_root_key_waited(volume: &str) -> String {
    format!("{volume} waits for its key file at boot, which this installer does not configure")
}

pub fn custom_keep_boot(bootloader: &str) -> String {
    match bootloader {
        "systemd" => "systemd-boot does not use a separate /boot".to_string(),
        _ => format!("{bootloader} installs with a supported ext4 /boot"),
    }
}

pub fn erasing(disk: &str) -> String {
    format!("Everything on {disk} will be erased. Are you sure?")
}

pub fn changing_partitions(disk: &str) -> String {
    format!("Partitions marked format on {disk} will be erased. Are you sure?")
}

pub fn removing_partitions(disk: &str, count: usize) -> String {
    match count {
        1 => format!("1 partition on {disk} will be DELETED and its contents lost. Are you sure?"),
        count => {
            format!("{count} partitions on {disk} will be DELETED and their contents lost. Are you sure?")
        }
    }
}

pub fn summary_deleted(partition: &str) -> String {
    format!("{partition}  DELETED, contents lost")
}
pub fn summary_created(gb: u64, fstype: &str) -> String {
    format!("a new {gb} GB partition, format as {fstype}")
}
pub const ROW_REMOVED: &str = "removed";
pub const ROW_CREATED: &str = "created";

pub const INSTALLATION_SUMMARY: &str = "Installation Summary";
pub const READY: &str = "Ready to install?";
pub const START_INSTALLATION: &str = "Start Installation";

pub const INSTALL_KEYS: &str = "\u{2191}\u{2193} navigate \u{2022} \u{23ce}  select";

pub const INSTALL: &str = "Install";
pub const SHUT_DOWN: &str = "Shut down";
pub const GO_BACK: &str = "Go back";
pub const EXIT_SHELL: &str = "Switch to shell";

pub fn switch_note(vt: u32) -> String {
    format!("The installer is running on tty{vt}. To return to the installer press Ctrl+Alt+F{vt}.")
}
pub const ROW_CONFIRM: &str = "password (confirm)";
pub const ROW_LAYOUT: &str = "partition layout";
pub const LAYOUT_WHOLE: &str = "whole disk";
pub const LAYOUT_MANUAL: &str = "manual";

pub fn layout_headings() -> [&'static str; 5] {
    ["size", "filesystem", "format", "type", "mount"]
}
pub const ASSIGN: &str = "Assign";
pub const UNASSIGN: &str = "Unassign";
pub const FORMAT_ROW: &str = "Format";
pub const SELECT_FORMAT: &str = "select partition format";
pub const USE_DISK: &str = "use this disk";
pub const USE_DISK_ACTION: &str = "Use this disk";
pub const CHOOSE_KEYS: &str =
    "\u{2191}\u{2193} navigate \u{2022} \u{23ce}  confirm \u{2022} Esc back";
pub const RESET_CHANGES: &str = "Reset changes";
pub const DELETE_PART: &str = "Delete partition";
pub const CLEAR_PARTS: &str = "Clear partitions";
pub const CREATE_PART: &str = "Create partition";
pub const NEW_PARTITION: &str = "create partition";
pub const NEW_SIZE: &str = "size";
pub const CELL_REMOVE: &str = "remove";
pub const CELL_NEW: &str = "new";
pub fn clear_all(count: usize) -> String {
    format!("plan to remove all {count} partitions on this disk?")
}
pub const OPEN_ENCRYPTED: &str = "Open Encrypted Partition";
pub const CLOSE_ENCRYPTED: &str = "Close Encrypted Partition";
pub const NO_FORMAT: &str = "Do not format";
pub const FORMAT_TICK: &str = "\u{2713}";
pub const LUKS_OPEN: &str = "luks (open)";
pub const LUKS_CLOSED: &str = "luks (closed)";
pub const EFI_CELL: &str = "efi";

pub fn mount_points() -> [&'static str; 5] {
    ["/boot/efi", "/", "/var", "/var/home", "/swap"]
}

pub fn format_options() -> [&'static str; 5] {
    ["btrfs", "ext4", "fat32", "xfs", "luks"]
}

pub fn plain_formats() -> [&'static str; 4] {
    ["btrfs", "ext4", "fat32", "xfs"]
}

pub fn boot_chain(boot: &str) -> Option<&'static str> {
    match boot {
        "uki-shim" => Some("owner-signed UKI through Microsoft shim"),
        "uki-db" => Some("owner-signed UKI through firmware keys"),
        _ => None,
    }
}

pub fn size_said(size: &str) -> String {
    // lsblk prints binary units and the form states decimal gigabytes, so the two factors differ.
    let number: String = size
        .chars()
        .take_while(|letter| letter.is_ascii_digit() || *letter == '.')
        .collect();
    let unit: String = size.chars().skip(number.len()).collect();
    let Ok(number) = number.parse::<f64>() else {
        return size.to_string();
    };
    let bytes = match unit.as_str() {
        "K" => number * 1024.0,
        "M" => number * 1024.0 * 1024.0,
        "G" => number * 1024.0 * 1024.0 * 1024.0,
        "T" => number * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "" | " GB" => return format!("{number:.1} GB"),
        _ => return size.to_string(),
    };
    format!("{:.1} GB", bytes / 1_000_000_000.0)
}

pub fn home_too_small(size: &str) -> String {
    format!("{size} is below the 1.0 GB home minimum; keep system and home together")
}

pub fn custom_root_too_small(gb: u64, reserve: u64) -> String {
    format!("a {gb} GB root is below the {reserve} GB this image needs")
}

pub fn home_too_big(size: &str, disk: &str) -> String {
    format!("{size} of home leaves no room for the system on the {disk} disk; keep system and home together")
}

pub fn written_over(
    bootloader: &str,
    filesystem: &str,
    var: &str,
    encrypted: bool,
) -> Vec<(String, String)> {
    let mut rows = vec![("esp".to_string(), format!("{}  fat32", size_said("2")))];
    if bootloader != "systemd" {
        rows.push(("/boot".to_string(), format!("{}  ext4", size_said("2"))));
    }
    if !var.is_empty() {
        let size = size_said(var);
        rows.push((
            "home".to_string(),
            match encrypted {
                true => format!("{size}  {filesystem}  {DATA_ENCRYPTED}"),
                false => format!("{size}  {filesystem}"),
            },
        ));
    }
    rows.push(("root".to_string(), format!("the rest  {filesystem}")));
    rows
}
pub const ROW_DISK: &str = "disk";
pub const DISK_SELECTION: &str = "disk selection";
pub const ROW_HOSTNAME: &str = "computer name";
pub const ROW_ACCOUNT: &str = "username";
pub const ROW_PASSWORD: &str = "password";
pub const ROW_ENCRYPTION: &str = "encryption";
pub const ROW_PASSPHRASE: &str = "passphrase";
pub const ROW_PIN: &str = "PIN";
pub const ROW_DATA: &str = "home directory";
pub const ROW_SIZE: &str = "size";
pub const PASSWORD_SET: &str = "set";
pub const NOT_SET: &str = "not set";

pub fn still_needs(fields: &[&str]) -> String {
    format!("Missing: {}", fields.join(", "))
}

pub fn installing(image: &str) -> String {
    format!("{image} installer \u{2022} configure")
}

pub fn logging(log: Option<&std::path::Path>) -> String {
    match log {
        Some(at) => format!("the install log is at {}", at.display()),
        None => "nothing here is writable, so this screen is the only copy".to_string(),
    }
}

pub fn writing(log: Option<&std::path::Path>) -> String {
    match log {
        Some(at) => format!("no going back from here \u{2014} log: {}", at.display()),
        None => "no going back from here \u{2014} and this screen is the log".to_string(),
    }
}

pub fn recovery(key: &str) -> String {
    format!("{} {key}", write_down())
}

pub fn write_down() -> &'static str {
    "Write it down and save it somewhere safe."
}

pub const RECOVERY_HEADING: &str = "LUKS disk encryption recovery key:";
pub const NEXT_STEPS_HEADING: &str = "Next Steps:";

pub const LEAVING: &str = "Leave the installer?";
pub const LEAVE_BACK: &str = "Keep going";
pub const LEAVE_OVER: &str = "Start the form again";
pub const LEAVE_SHELL: &str = "Quit the installer";
pub const INSTALL_DONE: &str = "Installation Complete!";
pub const RESTART: &str = "Restart now";
pub const DONE_KEYS: &str = "enter to restart, esc to quit the installer";

pub fn next_steps_setup() -> &'static str {
    "Before the installed image can boot you will need to set secure-boot to 'setup mode'. Exact steps to enable 'setup mode' are vendor specific but may require you to set secure-boot to 'custom' then deleting the existing platform key (PK)."
}

pub fn shim_steps() -> &'static str {
    "On first boot, use MokManager to enrol EFI/BOOT/MOK.cer; no firmware change is needed."
}

pub const PANEL_IMAGE: &str = "OS Image";
pub const PANEL_FIRMWARE: &str = "Detected System Firmware";
pub const PANEL_REQUIRED: &str = "Required";
pub const SETUP_OS: &str = "OS setup";
pub const SETUP_DISK: &str = "Disk setup";
pub const FIRMWARE_NO_EFIVARS: &str = "no EFI variables; not a UEFI boot";
pub const FIRMWARE_SECURE: &str = "secure boot";
pub const FIRMWARE_ON: &str = "on";
pub const FIRMWARE_OFF: &str = "off";
pub const FIRMWARE_SETUP: &str = "off (setup mode)";
pub const FIRMWARE_UNREADABLE: &str = "state unreadable";
pub const PLATFORM_KEY: &str = "platform key";
pub const PLATFORM_KEY_SETUP: &str = "none (setup mode)";
pub const PLATFORM_KEY_PRESENT: &str = "an existing platform key is present";
pub const PLATFORM_KEY_OWNER: &str = "the owner key";
pub const PLATFORM_KEY_UNKNOWN: &str = "could not be read";
pub const UKI_NEEDS_UEFI: &str = "this image needs a UEFI boot, and none was found";
pub const UKI_DB_ENROLL: &str = "the image enrolls its owner key on first boot";
pub const UKI_DB_OWNER: &str = "the owner key is enrolled; nothing to change";

pub fn required_uki_db() -> [&'static str; 3] {
    [
        "Secure-boot with the systemd-boot bootloader requires the image's platform key to be set in this system's UEFI.",
        "",
        "The existing platform key will be deleted, so any existing OS on this system will no longer boot with secure-boot on.",
    ]
}

pub const ERASE_WARNING_SECURE_BOOT: &str = "secure boot: the platform key must be cleared";
pub const ERASE_WARNING_UEFI_SETUP: &str = "afterwards, in UEFI setup: Custom mode";

pub const NONE: &str = "none";

#[cfg(test)]
mod tests {
    #[test]
    fn the_cost_names_the_disk_and_asks() {
        let said = super::erasing("/dev/vda");
        assert!(said.contains("/dev/vda"), "{said}");
        assert!(said.ends_with('?'), "{said}");
    }

    #[test]
    fn every_string_here_is_one_short_line() {
        for line in include_str!("copy.rs").lines() {
            let Some(rest) = line.strip_prefix("pub const ") else {
                continue;
            };
            if !rest.ends_with("\";") {
                continue;
            }
            let name = rest.split_once(':').expect("a const name").0;
            if name.ends_with("_KEYS") {
                continue;
            }
            let text = rest.split_once("= \"").expect("a string literal").1;
            assert!(text.chars().count() - 2 < 60, "{line}");
        }
    }

    #[test]
    fn uki_chains_are_visible_before_and_after_install() {
        assert!(super::boot_chain("uki-shim").is_some_and(|chain| chain.contains("Microsoft shim")));
        assert!(!super::written_over("systemd", "ext4", "", false)
            .iter()
            .any(|(name, _)| name == "boot chain"));
        assert!(super::next_steps_setup().contains("setup mode"));
        assert!(
            super::next_steps_setup().contains("platform key"),
            "the step that clears a foreign key is named"
        );
        assert!(super::shim_steps().contains("EFI/BOOT/MOK.cer"));
    }

    #[test]
    fn a_pcr_policy_tpm2_unit_names_the_embedded_files() {
        let policy = super::tpm2_unit("root", "/etc/tect/tpm2-enroll-root.key", "u", true);
        assert!(policy.contains("--tpm2-pcrs=7"), "{policy}");
        assert!(
            policy.contains("--tpm2-public-key=/run/systemd/tpm2-pcr-public-key.pem"),
            "{policy}"
        );
        assert!(
            policy.contains("--tpm2-signature=/run/systemd/tpm2-pcr-signature.json"),
            "{policy}"
        );
        let other = super::tpm2_unit("root", "/etc/tect/tpm2-enroll-root.key", "u", false);
        assert!(other.contains("--tpm2-pcrs=7"), "{other}");
        assert!(!other.contains("tpm2-pcr-public-key"), "{other}");
    }

    #[test]
    fn a_tpm2_unit_runs_before_any_login_and_says_so() {
        let unit = super::tpm2_unit("root", "/etc/tect/tpm2-enroll-root.key", "u", false);
        assert!(
            unit.contains("Before=systemd-user-sessions.service"),
            "{unit}"
        );
        assert!(unit.contains("TimeoutStartSec=120"), "{unit}");
        assert!(unit.contains("StandardOutput=journal+console"), "{unit}");
        assert!(unit.contains("StandardError=journal+console"), "{unit}");
        assert!(
            unit.contains(&format!("ExecStartPre=/bin/echo '{}'", super::ENROLLING)),
            "{unit}"
        );
        assert!(
            super::ENROLLING.chars().count() < 60,
            "{}",
            super::ENROLLING
        );
    }

    #[test]
    fn a_created_var_says_when_it_is_encrypted() {
        let var = |encrypted: bool| {
            super::written_over("grub2", "ext4", "200 GB", encrypted)
                .into_iter()
                .find(|(name, _)| name == "home")
                .expect("a home row")
                .1
        };
        assert!(var(true).contains(super::DATA_ENCRYPTED), "{}", var(true));
        assert!(
            !var(false).contains(super::DATA_ENCRYPTED),
            "{}",
            var(false)
        );
        assert!(super::DATA_ENCRYPTED.chars().count() < 60);
    }
}
