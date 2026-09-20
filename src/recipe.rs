use super::*;
use std::io::Write as _;

pub(crate) fn set(value: &mut Json, key: &str, field: Json) {
    let Json::Object(fields) = value else { return };
    match fields.iter_mut().find(|(name, _)| name == key) {
        Some((_, held)) => *held = field,
        None => fields.push((key.to_string(), field)),
    }
}

pub(crate) fn unset(value: &mut Json, key: &str) {
    let Json::Object(fields) = value else { return };
    fields.retain(|(name, _)| name != key);
}

pub fn complete(recipe: &Path, answers: &Answers) -> Result<Json, String> {
    let raw =
        std::fs::read_to_string(recipe).map_err(|err| format!("{}: {err}", recipe.display()))?;
    let mut doc = Json::parse(&raw).map_err(|err| format!("{}: {err}", recipe.display()))?;
    set(&mut doc, "disk", Json::string(&answers.disk));
    set(&mut doc, "hostname", Json::string(&answers.hostname));
    set(
        &mut doc,
        "encryption",
        Json::object([
            ("type", Json::string(&answers.encryption.kind)),
            ("passphrase", Json::string(&answers.encryption.passphrase)),
            ("pin", Json::string(&answers.encryption.pin)),
        ]),
    );
    let mut user = match doc {
        Json::Object(ref mut fields) => match fields.iter().position(|(name, _)| name == "user") {
            Some(at) => fields.remove(at).1,
            None => Json::object([]),
        },
        _ => return Err(format!("{}: not an object", recipe.display())),
    };
    set(&mut user, "username", Json::string(&answers.user));
    set(
        &mut user,
        "password",
        Json::string(hashed(&answers.password)?),
    );
    set(&mut doc, "user", user);
    if let Some(layout) = &answers.layout {
        unset(&mut doc, "varDisk");
        let mut mounts: Vec<Json> = layout
            .mounts
            .iter()
            .map(|mount| {
                Json::object([
                    ("partition", Json::string(&mount.partition)),
                    (
                        "target",
                        Json::string(match mount.target.as_str() {
                            "/swap" => "swap",
                            target => target,
                        }),
                    ),
                    ("fstype", Json::string(&mount.fstype)),
                ])
            })
            .collect();
        for create in &layout.creates {
            mounts.push(Json::object([
                ("partition", Json::string(&create.device)),
                (
                    "target",
                    Json::string(match create.target.as_str() {
                        "/swap" => "swap",
                        target => target,
                    }),
                ),
                ("fstype", Json::string(&create.fstype)),
            ]));
        }
        for (name, open) in layout.mappers() {
            mounts.push(Json::object([
                ("partition", Json::string(&mapper_path(&name))),
                ("target", Json::string(&open.target)),
                ("fstype", Json::string("unformatted")),
            ]));
        }
        set(&mut doc, "customMounts", Json::array(mounts));
        return Ok(doc);
    }
    let encrypted = answers.encryption.kind != NONE;
    if !answers.data.size.is_empty() {
        let size = match size_gb(&answers.data.size) {
            Some(size) => format!("{size}GB"),
            None => answers.data.size.clone(),
        };
        let mut var = Json::object([("size", Json::string(&size))]);
        if encrypted {
            set(&mut var, "encrypt", Json::Bool(true));
        }
        set(&mut doc, "varDisk", var);
    }
    Ok(doc)
}

pub(crate) fn hashed(password: &str) -> Result<String, String> {
    let mut child = Command::new("openssl")
        .args(["passwd", "-6", "-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|err| format!("openssl: {err}, and it is what hashes the password"))?;
    child
        .stdin
        .take()
        .ok_or("openssl: no stdin")?
        .write_all(format!("{password}\n").as_bytes())
        .map_err(|err| format!("openssl: {err}"))?;
    let out = child
        .wait_with_output()
        .map_err(|err| format!("openssl: {err}"))?;
    let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match out.status.success() && hash.starts_with('$') {
        true => Ok(hash),
        false => Err(format!(
            "openssl passwd wrote no crypt string, and a plaintext password loses the install at \
             its last step: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

pub(crate) struct Staged(pub(crate) PathBuf);

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(crate) fn stage(recipe: &Json) -> Result<Staged, String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let path = std::env::temp_dir().join(format!("bootc-anywhere.{}.json", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .map_err(|err| format!("{}: {err}", path.display()))?;
    let staged = Staged(path);
    file.write_all(recipe.render().as_bytes())
        .map_err(|err| format!("{}: {err}", staged.0.display()))?;
    Ok(staged)
}
