use common::prompt::Prompt;
use installer::PROGRAM;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE_ERROR: u8 = 1;

const FLAGS: &[&str] = &[
    "from",
    "disk",
    "hostname",
    "user",
    "password",
    "encryption",
    "passphrase",
    "pin",
];

struct Args {
    words: Vec<String>,
    given: Vec<(String, String)>,
}

impl Args {
    fn parse(words: Vec<String>) -> Result<Self, String> {
        let mut args = Args {
            words,
            given: Vec::new(),
        };
        for flag in FLAGS {
            let mut i = 0;
            while i < args.words.len() {
                let taken = if let Some(value) = args.words[i].strip_prefix(&format!("--{flag}=")) {
                    args.given.push(((*flag).to_string(), value.to_string()));
                    1
                } else if args.words[i] == format!("--{flag}") {
                    let value = args
                        .words
                        .get(i + 1)
                        .ok_or_else(|| format!("`--{flag}` takes a value"))?;
                    args.given.push(((*flag).to_string(), value.clone()));
                    2
                } else {
                    i += 1;
                    continue;
                };
                args.words.drain(i..i + taken);
            }
        }
        Ok(args)
    }

    fn switch(&mut self, flag: &str) -> bool {
        let before = self.words.len();
        self.words.retain(|word| word != &format!("--{flag}"));
        self.words.len() != before
    }

    fn flag(&self, flag: &str) -> Option<String> {
        self.given
            .iter()
            .rev()
            .find(|(name, _)| name == flag)
            .map(|(_, value)| value.clone())
    }
}

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

fn usage() -> String {
    format!(
        "{PROGRAM} — install the image this media carries onto a disk\n\n\
         usage: {PROGRAM} [--from <root>] [--disk <device>] [--hostname <name>]\n\
         \x20                     [--user <name>] [--password <password>]\n\
         \x20                     [--encryption <kind>] [--passphrase <passphrase>]\n\
         \x20                     [--pin <pin>]\n\n\
         Every flag is also a question on the screen. `--from` names the payload\n\
         root, which defaults to the media this binary was booted from.\n"
    )
}

fn run() -> Result<ExitCode, String> {
    let mut args = Args::parse(std::env::args().skip(1).collect())?;
    if args.words == ["--version"] {
        println!("{PROGRAM} v{}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    let prompt = Prompt::new(args.switch("no-tui"));
    if matches!(
        args.words.first().map(String::as_str),
        Some("-h" | "--help")
    ) {
        print!("{}", usage());
        return Ok(ExitCode::SUCCESS);
    }
    if let [word, ..] = args.words.as_slice() {
        return Err(format!("`{PROGRAM}` does not take {word}"));
    }

    let _lock = installer::hold()?;
    let root = match args.flag("from") {
        Some(from) => PathBuf::from(from),
        None => installer::root()?,
    };
    let found = installer::classify(&root)?;
    let payload = found.payload()?;
    eprintln!("{PROGRAM}: {}, from {}", payload.image, root.display());
    installer::own_screen(payload, &prompt);
    let given = installer::Given {
        disk: args.flag("disk"),
        hostname: args.flag("hostname"),
        user: args.flag("user"),
        password: args.flag("password"),
        encryption: args.flag("encryption"),
        passphrase: args.flag("passphrase"),
        pin: args.flag("pin"),
    };
    let Some(mut answers) = installer::Answers::collect(payload, given, &prompt)? else {
        return Ok(ExitCode::SUCCESS);
    };
    installer::run(payload, &mut answers, &prompt)?;
    Ok(ExitCode::SUCCESS)
}
