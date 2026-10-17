use super::*;

pub const RECIPE: &str = "install-recipe.json";

pub const IMAGE_ID: &str = "image-id";

pub fn media() -> PathBuf {
    Path::new(crate::BUILD_MANIFEST)
        .parent()
        .unwrap_or(Path::new("/"))
        .to_path_buf()
}

#[derive(Debug)]
pub struct Payload {
    pub recipe: PathBuf,
    pub(crate) install: InstallRecipe,
    pub image: String,
    pub image_id: Option<ImageId>,
    pub hostname: String,
    pub filesystem: String,
    pub bootloader: String,
    pub boot: String,
    pub composefs: bool,
    pub(crate) reserve: std::sync::OnceLock<u64>,
    pub luks_initramfs: bool,
    pub(crate) esp_entries: std::sync::OnceLock<Vec<String>>,
}

pub(crate) const ESP_LIST: &str = "for d in \"${1:-/usr/lib/efi}\"/*/*/EFI/*/ \"${2:-/boot/EFI}\"/*/; do if [ -d \"$d\" ]; then printf '%s\\n' \"${d%/}\"; fi; done";

impl Payload {
    pub(crate) fn esp_entries(&self) -> &[String] {
        self.esp_entries.get_or_init(|| {
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
                .arg(&self.image)
                .args(["/bin/sh", "-c", ESP_LIST])
                .output();
            match out {
                Ok(out) if out.status.success() => {
                    esp::entry_names(&String::from_utf8_lossy(&out.stdout))
                }
                _ => Vec::new(),
            }
        })
    }

    pub fn reserve_gb(&self) -> u64 {
        *self
            .reserve
            .get_or_init(|| root_reserve(&image_gb(&self.image)))
    }
}

pub(crate) fn root_reserve(image_gb: &Option<u64>) -> u64 {
    image_gb.map_or(10, |image| 2 * image + 2)
}

fn image_gb(image: &str) -> Option<u64> {
    let out = inspect(image, "{{.Size}}").output().ok()?;
    match out.status.success() {
        true => image_size(&String::from_utf8_lossy(&out.stdout)),
        false => None,
    }
}

fn image_size(raw: &str) -> Option<u64> {
    raw.trim()
        .parse::<u64>()
        .ok()
        .map(|bytes| bytes.div_ceil(1_000_000_000))
}

#[cfg(feature = "fuzzing")]
pub fn fuzz_readers(text: &str) {
    let _ = labelled(text);
    let _ = image_size(text);
}

#[derive(Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "classify builds one value per run, so the large variant's size is paid once"
)]
pub enum Found {
    Image(Payload),
    Repo(PathBuf),
    Nothing(PathBuf),
}

impl Found {
    pub fn payload(&self) -> Result<&Payload, String> {
        match self {
            Self::Image(payload) => Ok(payload),
            Self::Repo(root) => Err(format!(
                "{} is a repository, so there is nothing here to install yet\n\nhelp: \
                 `buildc build` in it, then `{PROGRAM} --from <the built payload>`",
                root.display()
            )),
            Self::Nothing(root) => Err(format!(
                "{} holds no {RECIPE} and no {}, so it is neither a payload nor a repository",
                root.display(),
                crate::REPO_FILE
            )),
        }
    }
}

pub fn classify(root: &Path) -> Result<Found, String> {
    let recipe = root.join(RECIPE);
    if recipe.is_file() {
        let install = InstallRecipe::read(&recipe)?;
        return Ok(Found::Image(Payload {
            image: install.image.clone(),
            image_id: recorded_id(root)?,
            hostname: install.hostname.clone(),
            filesystem: install.filesystem.clone(),
            bootloader: install.bootloader.clone(),
            boot: install.boot.clone(),
            composefs: install.composefs,
            reserve: std::sync::OnceLock::new(),
            luks_initramfs: install.luks_initramfs,
            esp_entries: std::sync::OnceLock::new(),
            recipe,
            install,
        }));
    }
    Ok(match root.join(crate::REPO_FILE).is_file() {
        true => Found::Repo(root.to_path_buf()),
        false => Found::Nothing(root.to_path_buf()),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageId(String);

impl ImageId {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let digits =
            text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        digits.then(|| Self(text.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn short(&self) -> &str {
        &self.0[..12]
    }
}

pub(crate) fn recorded_id(root: &Path) -> Result<Option<ImageId>, String> {
    let path = root.join(IMAGE_ID);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("{}: {err}", path.display())),
    };
    let id = raw.trim();
    ImageId::parse(id).map(Some).ok_or_else(|| {
        format!(
            "{}: `{id}` is not an image ID, which is 64 lowercase hex digits",
            path.display()
        )
    })
}

pub(crate) fn inspect(image: &str, format: &str) -> Command {
    let mut command = Command::new("podman");
    command.args(["image", "inspect", "--format", format, image]);
    command
}

pub(crate) fn same_image(
    image: &str,
    recorded: &ImageId,
    inspected: &std::process::Output,
) -> Result<(), String> {
    if !inspected.status.success() {
        return Err(format!(
            "podman cannot inspect {image}: {}",
            String::from_utf8_lossy(&inspected.stderr).trim()
        ));
    }
    let found = String::from_utf8_lossy(&inspected.stdout)
        .trim()
        .to_string();
    match found == recorded.as_str() {
        true => Ok(()),
        false => Err(format!(
            "{image} is image {found}, and this media was built to install {}; \
             rebuild the media with `buildc vm build iso`",
            recorded.as_str()
        )),
    }
}

pub(crate) fn read_back(image: &str) -> Command {
    let mut command = Command::new("podman");
    command.args([
        "save",
        "--quiet",
        "--format",
        "docker-archive",
        "--output",
        "/dev/stdout",
        image,
    ]);
    command
}

pub(crate) fn read_whole(
    image: &str,
    status: std::process::ExitStatus,
    stderr: &str,
) -> Result<(), String> {
    let said = stderr.trim();
    match (status.success(), said.contains("no space left on device")) {
        (true, _) => Ok(()),
        (false, true) => Err(format!(
            "reading {image} back needs free space as large as its largest layer, and the \
             temporary directory ran out: {said}\n\nhelp: set TMPDIR to a directory with \
             room, or use a machine with more memory, where this environment keeps /var/tmp"
        )),
        (false, false) => Err(format!(
            "{image} does not read back as the bytes it was stored with: {said}; rebuild \
             the media with `buildc vm build iso`"
        )),
    }
}

pub const LABEL: &str = "PAYLOAD";

pub(crate) const MOUNTPOINT: &str = "/run/buildc-payload";

pub fn root() -> Result<PathBuf, String> {
    let listed = Command::new("blkid")
        .args(["-t", &format!("LABEL={LABEL}"), "-o", "device"])
        .output();
    let devices = match &listed {
        Ok(out) => labelled(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => Vec::new(),
    };
    match devices.as_slice() {
        [] => Ok(media()),
        [device] => mounted(device),
        many => Err(format!(
            "{} partitions are labelled {LABEL}, so which one to install from is not \
             clear: {}\n\nhelp: `{PROGRAM} --from <root>` names one outright",
            many.len(),
            many.join(", ")
        )),
    }
}

pub(crate) fn labelled(listed: &str) -> Vec<String> {
    listed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

pub(crate) fn mounted(device: &str) -> Result<PathBuf, String> {
    let at = PathBuf::from(MOUNTPOINT);
    if at.join(RECIPE).is_file() {
        return Ok(at);
    }
    std::fs::create_dir_all(&at).map_err(|err| format!("{MOUNTPOINT}: {err}"))?;
    let out = Command::new("mount")
        .args(["-o", "ro", device, MOUNTPOINT])
        .output()
        .map_err(|err| format!("mount: {err}, and it is what reads a {LABEL} partition"))?;
    match out.status.success() {
        true => Ok(at),
        false => Err(format!(
            "mounting {device} at {MOUNTPOINT}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}
