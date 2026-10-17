use super::*;
use std::io::{BufRead as _, Write as _};

const TICK: std::time::Duration = std::time::Duration::from_millis(120);
const STORAGE_CONF: &str = "/etc/containers/storage.conf";

pub fn run(payload: &Payload, answers: &mut Answers, prompt: &Prompt) -> Result<(), String> {
    validate_recipe(payload)?;
    if !portable_name(&answers.user) {
        return Err(copy::account_name(&answers.user));
    }
    let password_hash = hashed(&answers.password)?;
    require_built_image(payload, prompt)?;
    require_signed_boot_chain(&payload.image, &payload.boot)?;
    if answers.opened == Opened::Tpm2 || answers.encryption.kind.starts_with("tpm2-") {
        require_tpm2_enrolment(&payload.image)?;
    }
    let mut prepared = prepare(payload, answers)?;
    let changes_table = prepared.layout.changes_table();
    installed(payload, answers, prompt, &password_hash, &mut prepared).map_err(|why| {
        match changes_table {
            true => format!("{why}\n\n{}", copy::table_already_changed(&answers.disk)),
            false => why,
        }
    })
}

fn require_built_image(payload: &Payload, prompt: &Prompt) -> Result<(), String> {
    match &payload.image_id {
        Some(recorded) => {
            let inspected = inspect(&payload.image, "{{.Id}}")
                .output()
                .map_err(|err| format!("podman: {err}, and it is what checks the image ID"))?;
            same_image(&payload.image, recorded, &inspected)?;
        }
        None if !prompt.draws() => eprintln!(
            "{PROGRAM}: {} {}",
            copy::PANEL_IMAGE_ID,
            copy::IMAGE_ID_UNRECORDED
        ),
        None => {}
    }
    let mut child = read_back(&payload.image)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("podman: {err}, and it is what reads the image back"))?;
    let pipe = child.stderr.take().expect("stderr was piped at spawn");
    let drained = std::thread::spawn(move || {
        let mut pipe = pipe;
        let mut said = String::new();
        std::io::Read::read_to_string(&mut pipe, &mut said).map(|_| said)
    });
    let mut region = match prompt.draws() {
        true => common::ui::Progress::open(copy::READING_BACK_FOOT).ok(),
        false => None,
    };
    match &mut region {
        Some(region) => {
            let _ = region.step(0, 100, copy::READING_BACK);
        }
        None => eprintln!("{PROGRAM}: {}", copy::READING_BACK),
    }
    let waited = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if let Some(region) = &mut region {
                    let _ = region.tick();
                }
                std::thread::sleep(TICK);
            }
            Err(err) => break Err(err),
        }
    };
    if let Some(region) = region {
        region.close();
    }
    let status = waited.map_err(|err| {
        let stopped = match child.kill() {
            Ok(()) => String::new(),
            Err(kill) => format!(", and stopping it failed too: {kill}"),
        };
        format!("podman: {err}, and it is what reads the image back{stopped}")
    })?;
    let stderr = drained
        .join()
        .map_err(|_| "podman: the thread reading its errors stopped".to_string())?
        .map_err(|err| format!("podman: {err}"))?;
    read_whole(&payload.image, status, &stderr)
}

fn installed(
    payload: &Payload,
    answers: &mut Answers,
    prompt: &Prompt,
    password_hash: &str,
    prepared: &mut Prepared,
) -> Result<(), String> {
    let karg = root_luks_arg(&prepared.layout, &payload.boot)?;
    let mut command = bootc_command(&payload.install, karg.as_deref());
    let (mut log, at) = open_log(payload);
    let mut stopped = None;
    if !prompt.draws() {
        eprintln!(
            "{PROGRAM}: installing {} as {} onto {}, {}",
            payload.image,
            answers.hostname,
            answers.disk,
            copy::logging(at.as_deref())
        );
    }
    let (events, writer) = std::io::pipe().map_err(|err| format!("podman: {err}"))?;
    let errors = writer.try_clone().map_err(|err| format!("podman: {err}"))?;
    let mut child = command
        .stdout(Stdio::from(writer))
        .stderr(Stdio::from(errors))
        .spawn()
        .map_err(|err| format!("podman: {err}, and it is what runs bootc"))?;
    drop(command);
    let mut region = match prompt.draws() {
        true => common::ui::Progress::open(&copy::writing(at.as_deref())).ok(),
        false => None,
    };
    if let Some(region) = &mut region {
        let _ = region.step(0, 100, "Installing the operating system");
    }
    {
        let (lines, arriving) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(events)
                .lines()
                .map_while(Result::ok)
            {
                if lines.send(line).is_err() {
                    return;
                }
            }
        });
        loop {
            let line = match arriving.recv_timeout(TICK) {
                Ok(line) => line,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(region) = &mut region {
                        let _ = region.tick();
                    }
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };
            if let Some(err) = log.as_mut().and_then(|log| writeln!(log, "{line}").err()) {
                log = None;
                let note = copy::log_stopped(&err.to_string());
                shown(&mut region, &note);
                stopped = Some(note);
            }
            shown(&mut region, &line);
        }
    }
    let finished = child.wait().map_err(|err| format!("podman: {err}"));
    if let Some(mut region) = region {
        if finished.as_ref().is_ok_and(|status| status.success()) {
            let _ = region.step(100, 0, "Operating system installed");
        }
        region.close();
    }
    if let Some(note) = &stopped {
        eprintln!("{PROGRAM}: {note}");
    }
    let status = finished?;
    if !status.success() {
        return Err(format!(
            "bootc did not finish: {status}, and {}",
            copy::logging(at.as_deref())
        ));
    }
    let notes = arrange(payload, answers, &prepared.layout, password_hash)?;
    configure_boot_chain(&payload.image, &answers.disk, &payload.boot)?;
    render_menu(&payload.image, &answers.disk)?;
    let steps = next_steps(&payload.boot, &firmware(payload));
    let recovery = prepared.recovery.take();
    let volumes = match recovery.is_some() {
        true => recovery_volumes(answers),
        false => Vec::new(),
    };
    let offered = match prompt.draws() {
        true => automatic::offered(payload, answers, recovery.as_deref()),
        false => Offered::None,
    };
    finish(
        recovery.as_deref(),
        at.as_deref(),
        steps,
        &notes,
        &volumes,
        offered,
        prompt,
    )
}

fn validate_recipe(payload: &Payload) -> Result<(), String> {
    if !Path::new(STORAGE_CONF).is_file() {
        return Err(format!(
            "{STORAGE_CONF}: missing, and bootc needs it to read the media store"
        ));
    }
    for store in &payload.install.stores {
        if !store.starts_with('/') || store.contains(':') || !Path::new(store).is_dir() {
            return Err(format!(
                "{}: `additionalImageStores` names unusable path {store:?}",
                payload.recipe.display()
            ));
        }
    }
    let held = Command::new("podman")
        .args(["image", "exists", &payload.install.image])
        .output()
        .map_err(|err| format!("podman: {err}, and it is what reads the media store"))?;
    match held.status.code() {
        Some(0) => Ok(()),
        Some(1) => Err(format!(
            "no store {STORAGE_CONF} names holds {}",
            payload.install.image
        )),
        _ => Err(format!(
            "podman could not read the stores {STORAGE_CONF} names: {}",
            String::from_utf8_lossy(&held.stderr).trim()
        )),
    }
}

pub(crate) fn bootc_command(recipe: &InstallRecipe, karg: Option<&str>) -> Command {
    let mut command = Command::new("podman");
    command.args([
        "run",
        "--rm",
        "--pull=never",
        "--privileged",
        "--pid=host",
        "--security-opt",
        "label=disable",
        "-v",
        "/dev:/dev",
        "-v",
        "/var/lib/containers:/var/lib/containers",
    ]);
    for store in &recipe.stores {
        command.args(["-v", &format!("{store}:{store}:ro")]);
    }
    command.args([
        "-v",
        &format!("{STORAGE_CONF}:{STORAGE_CONF}:ro"),
        "-v",
        &format!("{SYSROOT}:/target"),
        &recipe.image,
        "bootc",
        "install",
        "to-filesystem",
        // The installer finalizes the target, so bootc must leave it unfinalized.
        "--skip-finalize",
        "--target-imgref",
        &recipe.target_imgref,
    ]);
    if recipe.composefs {
        command.arg("--composefs-backend");
    }
    if !recipe.bootloader.is_empty() && recipe.bootloader != "grub2" {
        command.args(["--bootloader", &recipe.bootloader]);
    }
    if recipe.generic {
        command.arg("--generic-image");
    }
    if let Some(karg) = karg {
        command.args(["--karg", karg]);
    }
    command.arg("/target");
    command
}

fn root_luks_arg(layout: &CustomLayout, boot: &str) -> Result<Option<String>, String> {
    // A UKI boot chain needs no LUKS name argument, because the booted system finds the root by its GPT type.
    if !boot.is_empty() {
        return Ok(None);
    }
    let Some((name, open)) = layout
        .mappers()
        .into_iter()
        .find(|(_, open)| open.target == "/")
    else {
        return Ok(None);
    };
    Ok(Some(format!(
        "rd.luks.name={}={name}",
        luks_uuid(&open.partition)?
    )))
}

fn recovery_volumes(answers: &Answers) -> Vec<String> {
    let mut volumes = vec![format!("{}: {}", copy::ENCRYPTED_DISK, answers.disk)];
    if let Some(layout) = &answers.layout {
        for open in &layout.opens {
            volumes.push(format!("{}: {}", copy::ENCRYPTED_PARTITION, open.partition));
        }
    }
    volumes
}

pub(crate) fn finish(
    recovery: Option<&str>,
    log: Option<&Path>,
    steps: Option<&str>,
    notes: &[String],
    volumes: &[String],
    offered: Offered,
    prompt: &Prompt,
) -> Result<(), String> {
    if !prompt.draws() {
        if let Some(key) = recovery {
            println!("\n{}", copy::recovery(key));
            for volume in volumes {
                println!("{volume}");
            }
        }
        if let Some(steps) = steps {
            println!("{}", copy::NEXT_STEPS_HEADING);
            println!("{steps}\n");
        }
        for note in notes {
            println!("{note}");
        }
        eprintln!("{PROGRAM}: {}", copy::logging(log));
        return Ok(());
    }
    let mut offered = offered;
    choosing(
        &mut offered,
        |offered| {
            let rows = done_rows(recovery, log, steps, notes, volumes, offered);
            let actions = done_actions(offered);
            common::ui::offer_over(copy::INSTALL_DONE, rows, &actions, copy::DONE_KEYS)
        },
        automatic::finalize,
        restart,
    )
}

pub(crate) fn done_actions(offered: &Offered) -> Vec<Choice> {
    let mut actions = Vec::new();
    match offered {
        Offered::Ready(_) => actions.push(Choice::new(copy::FINALIZE_AUTO, "")),
        Offered::Unavailable(why) => {
            actions.push(Choice::new(copy::FINALIZE_AUTO, why.clone()).unavailable())
        }
        Offered::None => actions.push(Choice::new(copy::RESTART, "")),
    }
    if !matches!(offered, Offered::None) {
        actions.push(Choice::new(copy::FINALIZE_MANUAL, ""));
    }
    actions
}

pub(crate) fn choosing(
    offered: &mut Offered,
    mut draw: impl FnMut(&Offered) -> Result<Option<usize>, String>,
    mut finalize: impl FnMut(&Ready) -> Result<(), String>,
    mut restart: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    loop {
        let Some(at) = draw(offered)? else {
            return Ok(());
        };
        if let (0, Offered::Ready(ready)) = (at, &*offered) {
            if let Err(why) = finalize(ready) {
                eprintln!("{PROGRAM}: {why}");
                *offered = Offered::Unavailable(copy::AUTO_FAILED.to_string());
                continue;
            }
        }
        return restart();
    }
}

pub(crate) fn done_rows(
    recovery: Option<&str>,
    log: Option<&Path>,
    steps: Option<&str>,
    notes: &[String],
    volumes: &[String],
    offered: &Offered,
) -> Vec<Choice> {
    let mut rows = Vec::new();
    if let Some(key) = recovery {
        rows.push(Choice::new(copy::RECOVERY_HEADING, "").heading());
        rows.push(Choice::new(copy::write_down(), "").content());
        rows.push(Choice::new("", ""));
        for volume in volumes {
            rows.push(Choice::new(volume.clone(), "").content());
        }
        rows.push(Choice::new("", ""));
        rows.push(Choice::new(key, "").content().tinted());
    }
    if let Some(steps) = steps {
        if !rows.is_empty() {
            rows.push(Choice::new("", ""));
        }
        rows.push(Choice::new(copy::NEXT_STEPS_HEADING, "").heading());
        for line in common::ui::table::wrap(steps, common::ui::ROW_ROOM) {
            rows.push(Choice::new(line, "").content());
        }
    }
    for note in notes {
        rows.push(Choice::new(note.clone(), "").content());
    }
    rows.push(Choice::new(copy::logging(log), "").content());
    if !matches!(offered, Offered::None) {
        rows.push(Choice::new("", ""));
        for (at, paragraph) in copy::automatic_explanation().into_iter().enumerate() {
            if at > 0 {
                rows.push(Choice::new("", ""));
            }
            for line in common::ui::table::wrap(paragraph, common::ui::ROW_ROOM) {
                rows.push(Choice::new(line, "").content());
            }
        }
        if let Offered::Ready(ready) = offered {
            let window = match ready.pin {
                true => copy::AUTO_WINDOW_PIN,
                false => copy::AUTO_WINDOW,
            };
            rows.push(Choice::new(window, "").warning().content());
        }
    }
    rows
}

pub(crate) fn restart() -> Result<(), String> {
    let out = Command::new("systemctl")
        .arg("reboot")
        .output()
        .map_err(|err| format!("systemctl: {err}, and it is what restarts a machine"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "systemctl reboot: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

pub fn own_screen(payload: &Payload, prompt: &Prompt) {
    if !prompt.draws() {
        return;
    }
    print!("\u{1b}[2J\u{1b}[H");
    let _ = std::io::stdout().flush();
    common::ui::own_screen(copy::installing(&image_name(&payload.image)));
}

fn shown(region: &mut Option<common::ui::Progress>, line: &str) {
    match region {
        None => println!("{line}"),
        Some(region) => {
            let _ = region.note(line);
        }
    }
}
