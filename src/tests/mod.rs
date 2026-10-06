pub(crate) use super::*;
use std::io::Write as _;

mod account;
mod answers;
mod automatic;
mod cli;
mod discover;
mod disks;
mod editor;
mod esp;
mod etcwrite;
mod fdisk;
mod form;
mod layout;
mod lock;
mod panel;
mod payload;
mod recipe;
mod run;
mod table;
mod volumes;

const MEASURED_DUMP: &str = "\
label: gpt
device: /dev/vda
unit: sectors
first-lba: 2048
last-lba: 409566
sector-size: 512

/dev/vda1 : start=        2048, size=       40960, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=\"one\"
/dev/vda2 : start=      124928, size=       20480, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=\"small\"
/dev/vda3 : start=       83968, size=       40960, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=\"three\"
";

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("bootc-anywhere-{name}.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch tree");
    root
}

fn require_sfdisk() {
    let output = Command::new("sfdisk")
        .arg("--version")
        .output()
        .expect("sfdisk is required by the disk tests");
    assert!(output.status.success(), "sfdisk --version failed");
}

fn a_payload() -> Payload {
    let payload = Payload {
        recipe: "/mnt/tect/install-recipe.json".into(),
        install: an_install_recipe(),
        image: "ghcr.io/buildc-org/deb2:latest".to_string(),
        hostname: "deb2".to_string(),
        filesystem: "ext4".to_string(),
        bootloader: "grub2".to_string(),
        boot: String::new(),
        composefs: false,
        reserve: std::sync::OnceLock::new(),
        luks_initramfs: true,
        esp_entries: std::sync::OnceLock::new(),
    };
    payload
        .reserve
        .set(10)
        .expect("a reserve nothing has read yet");
    payload
        .esp_entries
        .set(Vec::new())
        .expect("entries nothing has read yet");
    payload
}

fn an_install_recipe() -> InstallRecipe {
    InstallRecipe {
        image: "ghcr.io/buildc-org/deb2:latest".to_string(),
        target_imgref: "ghcr.io/buildc-org/deb2:latest".to_string(),
        composefs: false,
        generic: false,
        bootloader: "grub2".to_string(),
        filesystem: "ext4".to_string(),
        hostname: "deb2".to_string(),
        groups: vec!["sudo".to_string()],
        stores: vec!["/var/lib/buildc/store".to_string()],
        boot: String::new(),
        luks_initramfs: true,
    }
}

fn a_payload_writing(entries: &[&str]) -> Payload {
    let mut payload = a_payload();
    payload.esp_entries = std::sync::OnceLock::new();
    payload
        .esp_entries
        .set(entries.iter().map(|entry| entry.to_string()).collect())
        .expect("entries nothing has read yet");
    payload
}

fn part(device: &str, fstype: &str) -> Partition {
    Partition {
        device: device.to_string(),
        fstype: fstype.to_string(),
        ..Default::default()
    }
}

fn label(name: &str) -> Label {
    Label {
        name: name.to_string(),
        link: String::new(),
    }
}

fn scan_of(disks: &[(&str, &str)], parts: &[(&str, Vec<Partition>)]) -> Scan {
    Scan {
        unread: Vec::new(),
        disks: disks
            .iter()
            .map(|(device, detail)| DiskScan {
                device: device.to_string(),
                detail: detail.to_string(),
                partitions: parts
                    .iter()
                    .find(|(at, _)| at == device)
                    .map(|(_, parts)| parts.to_vec())
                    .unwrap_or_default(),
                carries: false,
                table: Ok(DiskTable::default()),
                labels: Vec::new(),
                keys: Discovered::default(),
            })
            .collect(),
    }
}

const EMITTED: &str = r#"{
  "image": "ghcr.io/buildc-org/deb2:latest",
  "targetImgref": "ghcr.io/buildc-org/deb2:latest",
  "composeFsBackend": true,
  "genericImage": true,
  "bootloader": "grub2",
  "filesystem": "ext4",
  "hostname": "deb2",
  "user": { "groups": ["sudo"] },
  "additionalImageStores": ["/var/lib/buildc/store"]
}"#;
