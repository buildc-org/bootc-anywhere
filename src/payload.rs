use super::*;

pub const RECIPE: &str = "install-recipe.json";

pub fn media() -> PathBuf {
    Path::new(crate::BUILD_MANIFEST)
        .parent()
        .unwrap_or(Path::new("/"))
        .to_path_buf()
}

#[derive(Debug)]
pub struct Payload {
    pub recipe: PathBuf,
    pub image: String,
    pub hostname: String,
    pub filesystem: String,
    pub bootloader: String,
    pub boot: String,
    pub composefs: bool,
    pub(crate) reserve: std::sync::OnceLock<u64>,
    pub luks_initramfs: bool,
}

impl Payload {
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
    let out = Command::new("podman")
        .args(["image", "inspect", "--format", "{{.Size}}"])
        .arg(image)
        .output()
        .ok()?;
    match out.status.success() {
        true => String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<u64>()
            .ok()
            .map(|bytes| bytes.div_ceil(1_000_000_000)),
        false => None,
    }
}

#[derive(Debug)]
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
                 `tect build` in it, then `{PROGRAM} --from <the built payload>`",
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
        let raw = std::fs::read_to_string(&recipe)
            .map_err(|err| format!("{}: {err}", recipe.display()))?;
        let doc = Json::parse(&raw).map_err(|err| format!("{}: {err}", recipe.display()))?;
        let field = |key: &str| {
            json::text(&doc, key)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("{}: no `{key}`", recipe.display()))
        };
        let told = |key: &str| json::text(&doc, key).unwrap_or_default();
        return Ok(Found::Image(Payload {
            image: field("image")?,
            hostname: field("hostname")?,
            filesystem: told("filesystem"),
            bootloader: told("bootloader"),
            boot: told("boot"),
            composefs: matches!(
                json::field(&doc, "composeFsBackend"),
                Some(Json::Bool(true))
            ),
            reserve: std::sync::OnceLock::new(),
            luks_initramfs: matches!(json::field(&doc, "luksInitramfs"), Some(Json::Bool(true))),
            recipe,
        }));
    }
    Ok(match root.join(crate::REPO_FILE).is_file() {
        true => Found::Repo(root.to_path_buf()),
        false => Found::Nothing(root.to_path_buf()),
    })
}

pub const LABEL: &str = "TECT";

pub(crate) const MOUNTPOINT: &str = "/run/tect-payload";

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
