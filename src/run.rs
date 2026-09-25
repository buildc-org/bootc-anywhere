use super::*;
use std::io::{BufRead as _, Write as _};

pub fn run(payload: &Payload, answers: &mut Answers, prompt: &Prompt) -> Result<(), String> {
    require_signed_boot_chain(&payload.image, &payload.boot)?;
    if answers.opened == Opened::Tpm2 {
        require_tpm2_enrolment(&payload.image)?;
    }
    complete(&payload.recipe, answers)?;
    if let Some(layout) = answers.layout.as_mut() {
        cut_partitions(layout)?;
    }
    if let Some(layout) = answers.layout.as_ref() {
        esp::remove(layout)?;
    }
    let _opened = open_volumes(answers.layout.as_ref())?;
    let staged = stage(&complete(&payload.recipe, answers)?)?;
    let (mut log, at) = open_log(payload);
    if !prompt.draws() {
        eprintln!(
            "{PROGRAM}: installing {} as {} onto {}, {}",
            payload.image,
            answers.hostname,
            answers.disk,
            copy::logging(at.as_deref())
        );
    }
    let (events, writer) = std::io::pipe().map_err(|err| format!("{BACKEND}: {err}"))?;
    let errors = writer
        .try_clone()
        .map_err(|err| format!("{BACKEND}: {err}"))?;
    let mut child = Command::new(BACKEND)
        .arg(&staged.0)
        .stdout(Stdio::from(writer))
        .stderr(Stdio::from(errors))
        .spawn()
        .map_err(|err| format!("{BACKEND}: {err}"))?;
    let mut region = match prompt.draws() {
        true => common::ui::Progress::open(&copy::writing(at.as_deref())).ok(),
        false => None,
    };
    let mut recovery = None;
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
            let event = Event::of(&line);
            if let Event::Recovery(key) = &event {
                recovery = Some(key.clone());
            }
            if let (Some(said), Some(log)) = (event.logged(), &mut log) {
                let _ = writeln!(log, "{said}");
            }
            match &mut region {
                None => println!("{}", event.say()),
                Some(region) => {
                    let _ = match &event {
                        Event::Step(pct, flight, what) => region.step(*pct, *flight, what),
                        Event::Done(message) => region.step(100, 0, message),
                        Event::Recovery(_) => Ok(()),
                        Event::Note(message) => region.note(message),
                        Event::Other(line) => region.note(line),
                    };
                }
            }
        }
    }
    let finished = child.wait().map_err(|err| format!("{BACKEND}: {err}"));
    if let Some(region) = region {
        region.close();
    }
    drop(staged);
    let status = finished?;
    if !status.success() {
        return Err(format!(
            "{BACKEND} did not finish: {status}, and {}",
            copy::logging(at.as_deref())
        ));
    }
    let notes = arrange(payload, answers)?;
    configure_boot_chain(&payload.image, &answers.disk, &payload.boot)?;
    render_menu(&payload.image, &answers.disk)?;
    let steps = next_steps(&payload.boot, &firmware(payload));
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
    loop {
        let rows = done_rows(recovery, log, steps, notes, volumes, &offered);
        let mut actions = Vec::new();
        match &offered {
            Offered::Ready(_) => actions.push(Choice::new(copy::FINALIZE_AUTO, "")),
            Offered::Unavailable(why) => {
                actions.push(Choice::new(copy::FINALIZE_AUTO, why.clone()).unavailable())
            }
            Offered::None => actions.push(Choice::new(copy::RESTART, "")),
        }
        if !matches!(offered, Offered::None) {
            actions.push(Choice::new(copy::FINALIZE_MANUAL, ""));
        }
        match common::ui::offer_over(copy::INSTALL_DONE, rows, &actions, copy::DONE_KEYS)? {
            None => return Ok(()),
            Some(at) => {
                if let (0, Offered::Ready(ready)) = (at, &offered) {
                    if let Err(why) = automatic::finalize(ready) {
                        eprintln!("{PROGRAM}: {why}");
                        offered = Offered::Unavailable(copy::AUTO_FAILED.to_string());
                        continue;
                    }
                }
                return restart();
            }
        }
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
