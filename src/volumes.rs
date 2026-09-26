use super::*;
use std::io::Write as _;

const LOG: &str = "bootc-anywhere.log";

const IN_RAM: &str = "/run";

pub(crate) fn open_log(payload: &Payload) -> (Option<std::fs::File>, Option<PathBuf>) {
    if payload.recipe.parent() == Some(Path::new(MOUNTPOINT)) && remounted_rw(MOUNTPOINT) {
        let path = Path::new(MOUNTPOINT).join(LOG);
        if let Ok(file) = std::fs::File::create(&path) {
            return (Some(file), Some(path));
        }
    }
    let path = Path::new(IN_RAM).join(LOG);
    match std::fs::File::create(&path) {
        Ok(file) => (Some(file), Some(path)),
        Err(_) => (None, None),
    }
}

fn remounted_rw(at: &str) -> bool {
    Command::new("mount")
        .args(["-o", "remount,rw", at])
        .output()
        .is_ok_and(|out| out.status.success())
}

pub(crate) fn mapper_path(name: &str) -> String {
    format!("/dev/mapper/{name}")
}

#[derive(Default)]
pub(crate) struct Mappers(Vec<String>);

impl Drop for Mappers {
    fn drop(&mut self) {
        for name in &self.0 {
            if let Err(why) = close_volume(name) {
                eprintln!("{PROGRAM}: {why}");
            }
        }
    }
}

pub(crate) fn open_volumes(layout: Option<&CustomLayout>) -> Result<Mappers, String> {
    let mut opened = Mappers::default();
    let Some(layout) = layout else {
        return Ok(opened);
    };
    for (name, open) in layout.mappers() {
        open_volume(open, &name)?;
        opened.0.push(name);
    }
    Ok(opened)
}

pub(crate) fn open_command(open: &LuksOpen, name: &str) -> Command {
    let mut command = Command::new("cryptsetup");
    command.args(["-q", "luksOpen"]);
    match &open.key {
        Key::Passphrase(_) | Key::Data(_) => {
            command.args(["--key-file", "-"]);
        }
        Key::File(path) => {
            command.args(["--key-file"]).arg(path);
        }
    }
    command.arg(&open.partition).arg(name);
    command
}

pub(crate) fn open_volume(open: &LuksOpen, name: &str) -> Result<(), String> {
    let mut command = open_command(open, name);
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    if open.key.bytes().is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("cryptsetup: {err}, and it is what opens {}", open.partition))?;
    if let Some(bytes) = open.key.bytes() {
        child
            .stdin
            .take()
            .ok_or("cryptsetup: no stdin")?
            .write_all(bytes)
            .map_err(|err| format!("cryptsetup: {err}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|err| format!("cryptsetup: {err}"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "cryptsetup could not open {}: {}",
            open.partition,
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

pub(crate) fn format_command(partition: &str) -> Command {
    let mut command = Command::new("cryptsetup");
    command
        .args(["-q", "luksFormat", "--type", "luks2", "--key-file", "-"])
        .arg(partition);
    command
}

pub(crate) fn luks_format(partition: &str, key: &[u8]) -> Result<(), String> {
    let mut child = format_command(partition)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("cryptsetup: {err}, and it is what encrypts {partition}"))?;
    child
        .stdin
        .take()
        .ok_or("cryptsetup: no stdin")?
        .write_all(key)
        .map_err(|err| format!("cryptsetup: {err}"))?;
    let out = child
        .wait_with_output()
        .map_err(|err| format!("cryptsetup: {err}"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "cryptsetup could not encrypt {partition}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

pub(crate) fn close_volume(name: &str) -> Result<(), String> {
    let out = Command::new("cryptsetup")
        .args(["-q", "close", name])
        .output()
        .map_err(|err| format!("cryptsetup: {err}, and it is what closes a container"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "{} is still open: {}",
            mapper_path(name),
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}
