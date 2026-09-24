use super::*;

pub(crate) fn is_fat(fstype: &str) -> bool {
    ["vfat", "fat", "fat32"].contains(&fstype)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Link {
    Uuid(String),
    Label(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Carried {
    pub(crate) name: String,
    pub(crate) links: Vec<Link>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Label {
    pub(crate) name: String,
    pub(crate) link: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EspRemoval {
    pub(crate) partition: String,
    pub(crate) entries: Vec<String>,
}

const MICROSOFT: &str = "Microsoft";

pub(crate) fn entries(at: &Path) -> Vec<Carried> {
    let Some(efi) = inside(at, Path::new("EFI")).filter(|path| path.is_dir()) else {
        return Vec::new();
    };
    let Ok(listed) = std::fs::read_dir(&efi) else {
        return Vec::new();
    };
    let entries = loader_links(at);
    let mut found: Vec<Carried> = listed
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| !name.eq_ignore_ascii_case("BOOT"))
        .map(|name| {
            let links = vendor_links(&efi, &name);
            let fallback = links.is_empty() && !name.eq_ignore_ascii_case(MICROSOFT);
            Carried {
                name,
                links: match fallback {
                    true => entries.clone(),
                    false => links,
                },
            }
        })
        .collect();
    found.sort_by(|one, other| one.name.cmp(&other.name));
    found
}

fn vendor_links(efi: &Path, name: &str) -> Vec<Link> {
    let dir = efi.join(name);
    let mut links = Vec::new();
    if let Some(uuid) = read(&dir, "bootuuid.cfg").and_then(|text| bootuuid(&text)) {
        links.push(Link::Uuid(uuid));
    }
    if let Some(link) = read(&dir, "grub.cfg").and_then(|text| searched(&text)) {
        links.push(link);
    }
    links
}

fn read(dir: &Path, name: &str) -> Option<String> {
    let file = inside_file(dir, Path::new(name))?;
    std::fs::read_to_string(&file).ok()
}

pub(crate) fn bootuuid(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let said = line.trim().strip_prefix("set BOOT_UUID=")?;
        named(said)
    })
}

pub(crate) fn searched(text: &str) -> Option<Link> {
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let command = words.next().unwrap_or("");
        let rest: Vec<&str> = words.collect();
        if command == "search.fs_uuid" {
            if let Some(uuid) = rest.first().and_then(|value| named(value)) {
                return Some(Link::Uuid(uuid));
            }
        }
        if command != "search" {
            continue;
        }
        for (at, word) in rest.iter().enumerate() {
            if !matches!(*word, "--fs-uuid" | "--label") {
                continue;
            }
            let value = name_after(rest.get(at + 1..).unwrap_or_default());
            match (*word, value) {
                ("--fs-uuid", Some(uuid)) => return Some(Link::Uuid(uuid)),
                ("--label", Some(label)) => return Some(Link::Label(label)),
                _ => {}
            }
        }
    }
    None
}

fn name_after(words: &[&str]) -> Option<String> {
    let mut at = 0;
    while at < words.len() {
        let word = words[at];
        at += 1;
        if word == "--set" {
            if words.get(at).is_some_and(|value| !value.starts_with("--")) {
                at += 1;
            }
            continue;
        }
        if word.starts_with("--") {
            continue;
        }
        return named(word);
    }
    None
}

fn named(said: &str) -> Option<String> {
    let value = said.trim().trim_matches(['"', '\'']);
    (!value.is_empty() && !value.contains('$')).then(|| value.to_string())
}

/// Reads what the loader entries on one filesystem boot, in the order the
/// search runs: the root's UUID first, then the container the root lives in.
fn loader_links(at: &Path) -> Vec<Link> {
    for file in loader_files(at) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let links = bls_links(&text);
        if !links.is_empty() {
            return links;
        }
    }
    Vec::new()
}

pub(crate) fn loader_files(at: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in ["loader/entries", "boot/loader/entries"] {
        let Some(entries) = inside(at, Path::new(dir)).filter(|path| path.is_dir()) else {
            continue;
        };
        let Ok(listed) = std::fs::read_dir(&entries) else {
            continue;
        };
        let mut files: Vec<PathBuf> = listed
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|kind| kind == "conf"))
            .filter_map(|entry| inside_file(at, &Path::new(dir).join(entry.file_name())))
            .collect();
        files.sort();
        found.extend(files);
    }
    found
}

pub(crate) fn bls_links(text: &str) -> Vec<Link> {
    let words: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("options "))
        .flat_map(str::split_whitespace)
        .collect();
    let mut links: Vec<Link> = words
        .iter()
        .filter_map(|word| word.strip_prefix("root=UUID="))
        .map(|uuid| Link::Uuid(uuid.to_string()))
        .collect();
    links.extend(
        words
            .iter()
            .filter_map(|word| word.strip_prefix("rd.luks.uuid="))
            .map(|uuid| Link::Uuid(uuid.strip_prefix("luks-").unwrap_or(uuid).to_string())),
    );
    links
}

pub(crate) fn entry_names(listed: &str) -> Vec<String> {
    let mut names: Vec<String> = listed
        .lines()
        .filter_map(|line| line.trim().rsplit('/').next())
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("BOOT"))
        .map(str::to_string)
        .collect();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup_by(|one, other| one.eq_ignore_ascii_case(other));
    names
}

pub(crate) fn linked(
    walked: Vec<(String, Vec<Carried>)>,
    partitions: &[Partition],
    windows: Option<&str>,
) -> Vec<(String, Vec<Label>)> {
    walked
        .into_iter()
        .map(|(at, carried)| {
            let rows = carried
                .into_iter()
                .map(|carried| {
                    let link = carried.links.iter().find_map(|link| match link {
                        Link::Uuid(uuid) => partitions.iter().find(|part| {
                            !part.uuid.is_empty() && part.uuid.eq_ignore_ascii_case(uuid)
                        }),
                        Link::Label(label) => partitions.iter().find(|part| {
                            !part.label.is_empty() && part.label.eq_ignore_ascii_case(label)
                        }),
                    });
                    let link = match link {
                        Some(part) => part.device.clone(),
                        None if carried.name.eq_ignore_ascii_case(MICROSOFT) => {
                            windows.unwrap_or_default().to_string()
                        }
                        None => String::new(),
                    };
                    Label {
                        name: carried.name,
                        link,
                    }
                })
                .collect();
            (at, rows)
        })
        .collect()
}

pub(crate) fn removals(scan: &Scan, layout: &CustomLayout) -> Vec<EspRemoval> {
    let mut found = Vec::new();
    for disk in &scan.disks {
        for (at, labels) in &disk.labels {
            let Some(partition) = disk.partitions.iter().find(|part| &part.device == at) else {
                continue;
            };
            let receives = layout
                .answer(partition)
                .is_some_and(|mounted| mounted.target == "/boot/efi");
            if !is_fat(&partition.fstype) || !receives || layout.formatted(at) {
                continue;
            }
            let entries: Vec<String> = labels
                .iter()
                .filter(|label| layout.entry_gone(&label.link))
                .map(|label| label.name.clone())
                .collect();
            if !entries.is_empty() {
                found.push(EspRemoval {
                    partition: at.clone(),
                    entries,
                });
            }
        }
    }
    found
}

pub(crate) fn remove(layout: &CustomLayout) -> Result<(), String> {
    for removal in &layout.esp {
        let mut mounts = Mounts::default();
        let at = mount_rw(&removal.partition, &mut mounts)?;
        let efi = at.join("EFI");
        for entry in &removal.entries {
            remove_vendor(&efi, entry)
                .map_err(|why| format!("{why}\n\n{}", copy::table_already_changed(&layout.disk)))?;
        }
        mounts.release(&at)?;
    }
    Ok(())
}

pub(crate) fn remove_vendor(efi: &Path, entry: &str) -> Result<(), String> {
    let name = Path::new(entry);
    let one = name.components().count() == 1
        && matches!(
            name.components().next(),
            Some(std::path::Component::Normal(_))
        );
    if !one {
        return Err(copy::esp_entry_name(entry));
    }
    let said = efi.join(name);
    let meta = std::fs::symlink_metadata(&said)
        .map_err(|err| copy::esp_entry_missing(&said.display().to_string(), &err.to_string()))?;
    let removed = match meta.is_dir() {
        true => std::fs::remove_dir_all(&said),
        false => std::fs::remove_file(&said),
    };
    removed.map_err(|err| copy::esp_entry_remove(&said.display().to_string(), &err.to_string()))
}
