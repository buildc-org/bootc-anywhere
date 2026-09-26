use clap::Parser;
use common::prompt::Prompt;
use installer::cli::Args;
use installer::PROGRAM;
use std::process::ExitCode;

const USAGE_ERROR: u8 = 1;

fn main() -> ExitCode {
    // SAFETY: SIG_DFL installs no handler, so no handler can run asynchronously against this process.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };

    match run() {
        Ok(code) => code,
        Err(message) => {
            let stop = match message.ends_with(['.', '!', '?']) || message.contains('\n') {
                true => "",
                false => ".",
            };
            eprintln!("Error: {message}{stop}");
            ExitCode::from(USAGE_ERROR)
        }
    }
}

fn refused(error: clap::Error) -> ExitCode {
    use clap::error::ErrorKind;
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            print!("{error}");
            ExitCode::SUCCESS
        }
        _ => {
            let message = error.to_string();
            let message = message.strip_prefix("error: ").unwrap_or(&message);
            eprintln!("Error: {message}");
            ExitCode::from(USAGE_ERROR)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let words: Vec<String> = std::env::args().skip(1).collect();
    if words == ["--version"] {
        println!("{PROGRAM} v{}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    let args = match Args::try_parse_from(std::iter::once(PROGRAM.to_string()).chain(words)) {
        Ok(args) => args,
        Err(error) => return Ok(refused(error)),
    };
    let prompt = Prompt::new(args.no_tui);

    let _lock = installer::hold()?;
    let root = match args.from {
        Some(from) => from,
        None => installer::root()?,
    };
    let found = installer::classify(&root)?;
    let payload = found.payload()?;
    eprintln!("{PROGRAM}: {}, from {}", payload.image, root.display());
    installer::own_screen(payload, &prompt);
    let given = installer::Given {
        disk: args.disk,
        hostname: args.hostname,
        user: args.user,
        password: args.password,
        encryption: args.encryption,
        passphrase: args.passphrase,
        pin: args.pin,
    };
    let Some(mut answers) = installer::Answers::collect(payload, given, &prompt)? else {
        return Ok(ExitCode::SUCCESS);
    };
    installer::run(payload, &mut answers, &prompt)?;
    Ok(ExitCode::SUCCESS)
}
