use super::*;

const LOCK: &str = "/run/bootc-anywhere.lock";

pub(crate) fn lock() -> PathBuf {
    std::env::var_os("BUILDC_INSTALLER_LOCK").map_or_else(|| PathBuf::from(LOCK), PathBuf::from)
}

pub(crate) fn console(pid: libc::pid_t) -> String {
    name(std::fs::read_link(format!("/proc/{pid}/fd/0")).ok())
}

pub(crate) fn name(tty: Option<PathBuf>) -> String {
    let is_console = |tty: &Path| {
        tty.parent()
            .is_some_and(|parent| parent.as_os_str() == "/dev")
            && tty
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("tty") || name == "console")
    };
    match tty {
        Some(tty) if is_console(&tty) => tty.display().to_string(),
        Some(tty) if tty.starts_with("/dev/pts/") => {
            format!("another session ({})", tty.display())
        }
        _ => "another console".into(),
    }
}

pub fn hold() -> Result<std::fs::File, String> {
    hold_at(&lock())
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "`struct flock` holds the small `c_int` lock and seek constants as `c_short`"
)]
pub(crate) fn hold_at(path: &Path) -> Result<std::fs::File, String> {
    use std::os::fd::AsRawFd as _;

    let file = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|why| {
            format!(
                "{}: {why}\nset $BUILDC_INSTALLER_LOCK to somewhere writable to run this without root",
                path.display()
            )
        })?;
    let wrlck = || libc::flock {
        l_type: libc::F_WRLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    for _ in 0..2 {
        // SAFETY: `file` stays open for the call and `wrlck()` is a fully initialised `flock` that fcntl only reads.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &wrlck()) } != -1 {
            return Ok(file);
        }
        let denied = std::io::Error::last_os_error().raw_os_error();
        if denied != Some(libc::EACCES) && denied != Some(libc::EAGAIN) {
            return Err(format!(
                "{}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        let mut holder = wrlck();
        // SAFETY: `file` stays open for the call and `holder` is a fully initialised `flock` that fcntl may write.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut holder) } == -1 {
            break;
        }
        if holder.l_type != libc::F_UNLCK as libc::c_short {
            return Err(format!(
                "the installer is running on {}; run `{PROGRAM}` again once it finishes",
                console(holder.l_pid)
            ));
        }
    }
    Err(format!(
        "the installer is running on another console; run `{PROGRAM}` again once it finishes"
    ))
}
