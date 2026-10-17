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
mod screens;
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

fn running_as_root() -> bool {
    // SAFETY: libc::geteuid has no precondition, so the call is sound.
    unsafe { libc::geteuid() == 0 }
}

fn write_only(path: &Path, bytes: &[u8]) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o200)).unwrap();
}

fn readable(path: &Path) -> Vec<u8> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::read(path).unwrap()
}

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

const AN_IMAGE_ID: &str = "30ecbe1509090b38ef3260f3609d64d8261230464eaa4d235901db496e9647fb";

fn a_payload() -> Payload {
    a_payload_writing(&[])
}

fn a_payload_writing(entries: &[&str]) -> Payload {
    let payload = Payload {
        recipe: "/mnt/buildc/install-recipe.json".into(),
        install: an_install_recipe(),
        image: "ghcr.io/buildc-org/deb2:latest".to_string(),
        image_id: ImageId::parse(AN_IMAGE_ID),
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
        .set(entries.iter().map(|entry| entry.to_string()).collect())
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

fn imported_store(name: &str) -> (PathBuf, PathBuf) {
    let root = scratch(name);
    let tree = root.join("tree/bin");
    std::fs::create_dir_all(&tree).expect("a layer tree");
    std::fs::write(tree.join("payload"), "the bytes the image was built with\n")
        .expect("a layer file");
    let tar = root.join("rootfs.tar");
    let packed = Command::new("tar")
        .args(["--owner=0", "--group=0", "-C"])
        .arg(root.join("tree"))
        .arg("-cf")
        .arg(&tar)
        .arg(".")
        .status()
        .expect("tar runs");
    assert!(packed.success(), "tar packs the layer");
    let conf = root.join("storage.conf");
    std::fs::write(
        &conf,
        format!(
            "[storage]\ndriver = \"vfs\"\ngraphroot = \"{0}/graph\"\n\
             rootless_storage_path = \"{0}/graph\"\nrunroot = \"{0}/run\"\n",
            root.display()
        ),
    )
    .expect("a storage.conf");
    let imported = Command::new("podman")
        .env("CONTAINERS_STORAGE_CONF", &conf)
        .args(["import", "--quiet"])
        .arg(&tar)
        .arg(STORED)
        .output()
        .expect("podman is required by the image check tests");
    assert!(
        imported.status.success(),
        "podman import: {}",
        String::from_utf8_lossy(&imported.stderr)
    );
    (root, conf)
}

const STORED: &str = "example.invalid/payload:1";

fn in_store(mut command: Command, conf: &Path) -> std::process::Output {
    command
        .env("CONTAINERS_STORAGE_CONF", conf)
        .output()
        .expect("podman runs")
}

fn alter_layer(root: &Path) {
    let layer_file = find_file(&root.join("graph"), "payload").expect("the stored layer file");
    let mut bytes = std::fs::read(&layer_file).expect("the stored layer file reads");
    bytes[4] ^= 1;
    std::fs::write(&layer_file, bytes).expect("the stored layer file is altered");
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() && !path.is_symlink() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|file| file == name) {
            return Some(path);
        }
    }
    None
}

fn remove_store(root: &Path) {
    let removed = Command::new("podman")
        .args(["unshare", "rm", "-rf"])
        .arg(root)
        .status()
        .expect("podman runs");
    assert!(removed.success(), "the scratch store is removed");
}

fn answered(kind: &str, passphrase: &str) -> Answers {
    Answers {
        disk: "/dev/vda".to_string(),
        hostname: "host".to_string(),
        user: "user".to_string(),
        password: "secret".to_string(),
        encryption: Encryption {
            kind: kind.to_string(),
            passphrase: passphrase.to_string(),
            pin: String::new(),
        },
        layout: None,
        opened: Opened::Keep,
    }
}
