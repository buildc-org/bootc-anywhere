use super::*;

#[derive(Clone)]
pub(crate) enum RowKind {
    Disk(String),
    Part {
        index: usize,
    },
    Created {
        index: usize,
    },
    Preview,
}

#[derive(Clone, Copy)]
pub(crate) enum PartAction {
    Assign,
    Format,
    Reset,
    Open,
    Close,
    Delete,
    Clear,
    Create,
    Rename,
}

pub(crate) fn effective_fs(partition: &Partition, answer: Option<&Mounted>) -> String {
    match answer {
        Some(mounted) if mounted.fstype != OPEN && mounted.fstype != "unformatted" => {
            mounted.fstype.clone()
        }
        _ => partition.fstype.clone(),
    }
}

pub(crate) fn fits(target: &str, fstype: &str) -> bool {
    match target {
        "/boot/efi" => is_fat(fstype),
        "/swap" => fstype == "swap",
        _ => LINUX_FILESYSTEMS.contains(&fstype),
    }
}

pub(crate) fn assigns(
    partition: &Partition,
    answer: Option<&Mounted>,
    boot: bool,
) -> Vec<&'static str> {
    let holds = effective_fs(partition, answer);
    copy::mount_points()
        .into_iter()
        .filter(|target| *target != "/boot" || boot)
        .filter(|target| fits(target, &holds))
        .collect()
}

pub(crate) fn open_points() -> [&'static str; 3] {
    ["/", "/var", "/var/home"]
}

fn assign_children(points: &[&'static str]) -> Vec<&'static str> {
    let mut children = points.to_vec();
    children.push(copy::UNASSIGN);
    children
}

pub(crate) fn mount_name(target: &str) -> &'static str {
    match target {
        "/boot/efi" => "EFI-SYSTEM",
        "/boot" => "boot",
        "/" => "root",
        "/var" | "/var/home" => "var",
        _ => "",
    }
}

pub(crate) fn rename_default(target: &str, label: &str) -> String {
    match label.is_empty() {
        true => mount_name(target).to_string(),
        false => label.to_string(),
    }
}

pub(crate) fn place_unassign(held: &mut Option<CustomLayout>, partition: &Partition) {
    if let Some(layout) = held.as_mut() {
        if let Some(open) = layout
            .opens
            .iter_mut()
            .find(|open| open.partition == partition.device)
        {
            open.target.clear();
        }
        if let Some(mount) = layout
            .mounts
            .iter_mut()
            .find(|mount| mount.partition == partition.device)
        {
            mount.target.clear();
        }
        layout.mounts.retain(|mount| {
            !(mount.partition == partition.device
                && mount.target.is_empty()
                && mount.fstype == "unformatted")
        });
    }
}

pub(crate) fn held_layout<'a>(
    held: &'a mut Option<CustomLayout>,
    disk: &str,
) -> &'a mut CustomLayout {
    held.get_or_insert_with(|| CustomLayout::empty(disk))
}

pub(crate) fn place_target(
    held: &mut Option<CustomLayout>,
    disk: &str,
    partition: &Partition,
    target: &str,
) {
    let layout = held_layout(held, disk);
    if let Some(open) = layout
        .opens
        .iter_mut()
        .find(|open| open.partition == partition.device)
    {
        open.target = target.to_string();
        return;
    }
    match layout
        .mounts
        .iter_mut()
        .find(|mount| mount.partition == partition.device)
    {
        Some(mount) => mount.target = target.to_string(),
        None => layout.mounts.push(CustomMount {
            partition: partition.device.clone(),
            target: target.to_string(),
            fstype: "unformatted".to_string(),
            passphrase: String::new(),
        }),
    }
}

pub(crate) fn place_format(
    held: &mut Option<CustomLayout>,
    disk: &str,
    partition: &Partition,
    fstype: &str,
) {
    let keep = fstype == copy::NO_FORMAT;
    let layout = held_layout(held, disk);
    if !keep {
        layout
            .opens
            .retain(|open| open.partition != partition.device);
    }
    let answer = match keep {
        true => "unformatted",
        false => fstype,
    };
    match layout
        .mounts
        .iter_mut()
        .find(|mount| mount.partition == partition.device)
    {
        Some(mount) => mount.fstype = answer.to_string(),
        None => layout.mounts.push(CustomMount {
            partition: partition.device.clone(),
            target: String::new(),
            fstype: answer.to_string(),
            passphrase: String::new(),
        }),
    }
    if let Some(at) = layout
        .mounts
        .iter()
        .position(|mount| mount.partition == partition.device)
    {
        let (target, fstype) = (
            layout.mounts[at].target.clone(),
            layout.mounts[at].fstype.clone(),
        );
        let carries = match keep {
            true => partition.fstype.as_str(),
            false => fstype.as_str(),
        };
        if !target.is_empty() && !fits(&target, carries) {
            layout.mounts[at].target.clear();
        }
    }
    if keep {
        layout
            .mounts
            .retain(|mount| !(mount.partition == partition.device && mount.target.is_empty()));
    }
}

#[derive(Default)]
pub(crate) struct TableData {
    pub(crate) rows: Vec<Vec<common::ui::Cell>>,
    pub(crate) selectable: Vec<bool>,
    pub(crate) menus: Vec<Vec<common::ui::MenuItem>>,
    pub(crate) actions: Vec<Vec<PartAction>>,
    pub(crate) kinds: Vec<RowKind>,
}

impl TableData {
    pub(crate) fn push(
        &mut self,
        cells: Vec<common::ui::Cell>,
        selectable: bool,
        menus: Vec<common::ui::MenuItem>,
        actions: Vec<PartAction>,
        kind: RowKind,
    ) {
        self.rows.push(cells);
        self.selectable.push(selectable);
        self.menus.push(menus);
        self.actions.push(actions);
        self.kinds.push(kind);
    }

    pub(crate) fn field(&self, cursor: usize, chosen: bool, focused: bool) -> common::ui::Field {
        common::ui::Field::table(
            copy::DISK_SELECTION,
            copy::ROW_DISK,
            &copy::layout_headings(),
            &copy::layout_widths(),
            self.rows.clone(),
            self.selectable.clone(),
            self.menus.clone(),
            cursor,
            focused,
            chosen,
        )
    }
}

pub(crate) fn layout_table(
    scan: &Scan,
    disk: &str,
    layout: Option<&CustomLayout>,
    payload: &Payload,
    var_size: &str,
    home: bool,
    encrypted: bool,
) -> TableData {
    let mut table = TableData::default();
    let boot = payload.bootloader != "systemd";
    for entry in &scan.disks {
        let chosen = entry.device == disk;
        let (size, rest) = match entry.detail.split_once("  ") {
            Some((size, rest)) => (size.to_string(), rest),
            None => (entry.detail.clone(), ""),
        };
        let model = rest
            .strip_suffix(copy::REMOVABLE)
            .map(str::trim_end)
            .filter(|model| !model.is_empty());
        let name = match model {
            Some(model) => format!("{model} ({})", entry.device),
            None => entry.device.clone(),
        };
        let radio = match chosen {
            true => "\u{25c9}",
            false => "\u{25cb}",
        };
        let choice = format!("{radio} {name}");
        let mut row = vec![
            common::ui::Cell::new(&choice),
            common::ui::Cell::new(&size),
            common::ui::Cell::new(""),
            common::ui::Cell::new(""),
            common::ui::Cell::new(""),
            common::ui::Cell::new(""),
        ];
        if chosen {
            row[0] = common::ui::Cell::set(&choice);
        }
        let (menus, actions) = match (layout.is_some(), chosen) {
            (true, true) => disk_menu(entry.partitions.len()),
            _ => (Vec::new(), Vec::new()),
        };
        table.push(
            row,
            true,
            menus,
            actions,
            RowKind::Disk(entry.device.clone()),
        );
        match layout {
            None if chosen => {
                for (name, size, filesystem, kind, mount) in
                    automatic_rows(payload, var_size, size_gb(&size), home, encrypted)
                {
                    table.push(
                        vec![
                            common::ui::Cell::new(&name),
                            common::ui::Cell::new(&size),
                            common::ui::Cell::new(&filesystem),
                            common::ui::Cell::new(""),
                            common::ui::Cell::new(&kind),
                            common::ui::Cell::new(&mount),
                        ],
                        false,
                        Vec::new(),
                        Vec::new(),
                        RowKind::Preview,
                    );
                }
            }
            Some(held) => {
                partition_rows(&mut table, entry, chosen.then_some(held), boot, payload);
                if chosen {
                    let devices = created_devices(
                        &entry.device,
                        &drawn_slots(&entry.partitions, &held.deletes),
                        held.creates.len(),
                    );
                    let last = held.creates.len().saturating_sub(1);
                    for (at, create) in held.creates.iter().enumerate() {
                        let branch = match at == last {
                            true => "\u{2514}\u{2500} ",
                            false => "\u{251c}\u{2500} ",
                        };
                        let device = devices.get(at).cloned().unwrap_or_default();
                        let (menus, actions) = created_menu(create, boot);
                        table.push(
                            created_cells(create, &device, branch),
                            true,
                            menus,
                            actions,
                            RowKind::Created { index: at },
                        );
                    }
                }
            }
            None => partition_rows(&mut table, entry, None, boot, payload),
        }
    }
    table
}

fn partition_rows(
    table: &mut TableData,
    disk: &DiskScan,
    held: Option<&CustomLayout>,
    boot: bool,
    payload: &Payload,
) {
    let parts = &disk.partitions;
    let chosen = held.is_some();
    let last = match chosen && !held.is_some_and(|held| held.creates.is_empty()) {
        true => usize::MAX,
        false => parts.len().saturating_sub(1),
    };
    for (at, partition) in parts.iter().enumerate() {
        let branch = match at == last {
            true => "\u{2514}\u{2500} ",
            false => "\u{251c}\u{2500} ",
        };
        let answer = held.and_then(|held| held.answer(partition));
        let carries = effective_fs(partition, answer.as_ref());
        let deleted = held.is_some_and(|held| held.deletes.contains(&partition.device));
        let renamed = held.and_then(|held| held.renamed(&partition.device));
        let cells = partition_cells(partition, answer, branch, deleted, renamed);
        let (menus, actions) = match held {
            Some(held) => partition_menu(partition, Some(held), boot),
            None => (Vec::new(), Vec::new()),
        };
        table.push(cells, chosen, menus, actions, RowKind::Part { index: at });
        let labels: Vec<&Label> = disk
            .labels
            .iter()
            .filter(|(device, _)| device == &partition.device)
            .flat_map(|(_, labels)| labels)
            .collect();
        let shown: Vec<(String, bool)> = match held.filter(|held| {
            is_fat(&carries)
                && held
                    .answer(partition)
                    .is_some_and(|mounted| mounted.target == "/boot/efi")
        }) {
            Some(held) => {
                let entries = payload.esp_entries();
                let mut rows: Vec<(String, bool)> = match held.formatted(&partition.device) {
                    true => Vec::new(),
                    false => labels
                        .iter()
                        .filter(|label| !held.entry_replaced(&label.link))
                        .filter(|label| {
                            !entries
                                .iter()
                                .any(|entry| entry.eq_ignore_ascii_case(&label.name))
                        })
                        .map(|label| (label.name.clone(), false))
                        .collect(),
                };
                rows.extend(entries.iter().map(|entry| (entry.clone(), true)));
                rows.sort_by(|one, other| one.0.cmp(&other.0));
                rows
            }
            None => labels
                .iter()
                .map(|label| (label.name.clone(), false))
                .collect(),
        };
        for (at, (label, written)) in shown.iter().enumerate() {
            let branch = match at + 1 == shown.len() {
                true => "   \u{2514}\u{2500} ",
                false => "   \u{251c}\u{2500} ",
            };
            let said = format!("{branch}{label}");
            let name = match written {
                true => common::ui::Cell::set(&said),
                false => common::ui::Cell::new(&said),
            };
            table.push(
                vec![
                    name,
                    common::ui::Cell::new(""),
                    common::ui::Cell::new(""),
                    common::ui::Cell::new(""),
                    common::ui::Cell::new(""),
                    common::ui::Cell::new(""),
                ],
                false,
                Vec::new(),
                Vec::new(),
                RowKind::Preview,
            );
        }
    }
}

pub(crate) fn size_gb(text: &str) -> Option<u64> {
    text.chars()
        .take_while(|letter| letter.is_ascii_digit() || *letter == '.')
        .collect::<String>()
        .parse::<f64>()
        .ok()
        .map(|number| number as u64)
}

pub(crate) fn automatic_rows(
    payload: &Payload,
    var_size: &str,
    disk: Option<u64>,
    home: bool,
    encrypted: bool,
) -> Vec<(String, String, String, String, String)> {
    let filesystem = payload.filesystem.clone();
    let boot = payload.bootloader != "systemd";
    let left = disk.and_then(|disk| disk.checked_sub(2 + if boot { 2 } else { 0 }));
    let separate = home;
    let home_size = match var_size.is_empty() {
        true => None,
        false => size_gb(var_size),
    };
    let root_size = match home_size {
        Some(home) => left.and_then(|left| left.checked_sub(home)),
        None if separate => None,
        None => left,
    };
    let said = |size: Option<u64>| match size {
        Some(size) => copy::size_said(&size.to_string()),
        None => "the rest".to_string(),
    };
    let mut rows: Vec<(String, String, String, String, String, bool)> = vec![(
        "EFI-SYSTEM".to_string(),
        copy::size_said("2"),
        "fat32".to_string(),
        "efi".to_string(),
        "/boot/efi".to_string(),
        false,
    )];
    if boot {
        rows.push((
            "boot".to_string(),
            copy::size_said("2"),
            "ext4".to_string(),
            "linux".to_string(),
            "/boot".to_string(),
            false,
        ));
    }
    let root = |inside| {
        (
            "root".to_string(),
            said(root_size),
            filesystem.clone(),
            "linux".to_string(),
            "/".to_string(),
            inside,
        )
    };
    let home = |inside| {
        (
            "var".to_string(),
            match home_size {
                Some(size) => copy::size_said(&size.to_string()),
                None => copy::size_said(var_size),
            },
            filesystem.clone(),
            "linux".to_string(),
            "/var/home".to_string(),
            inside,
        )
    };
    if encrypted {
        rows.push((
            "root (LUKS)".to_string(),
            said(left),
            String::new(),
            "luks".to_string(),
            String::new(),
            false,
        ));
        rows.push(root(true));
        if separate {
            rows.push(home(true));
        }
    } else {
        rows.push(root(false));
        if separate {
            rows.push(home(false));
        }
    }
    let last = |inside: bool| rows.iter().rposition(|(.., at)| *at == inside).unwrap_or(0);
    let (outside, inner) = (last(false), last(true));
    rows.into_iter()
        .enumerate()
        .map(|(at, (name, size, filesystem, kind, mount, inside))| {
            let glyph = match (inside, at == if inside { inner } else { outside }) {
                (false, true) => "\u{2514}\u{2500} ",
                (false, false) => "\u{251c}\u{2500} ",
                (true, true) => "   \u{2514}\u{2500} ",
                (true, false) => "   \u{251c}\u{2500} ",
            };
            (format!("{glyph}{name}"), size, filesystem, kind, mount)
        })
        .collect()
}

fn partition_cells(
    partition: &Partition,
    answer: Option<Mounted>,
    branch: &str,
    deleted: bool,
    renamed: Option<&str>,
) -> Vec<common::ui::Cell> {
    if deleted {
        let said = match partition.label.is_empty() {
            true => partition.device.clone(),
            false => format!("{} ({})", partition.label, partition.device),
        };
        return vec![
            common::ui::Cell::set(&format!("{branch}{said}")),
            common::ui::Cell::new(&copy::size_said(&partition.size)),
            common::ui::Cell::new(""),
            common::ui::Cell::set(copy::CELL_REMOVE),
            common::ui::Cell::new(""),
            common::ui::Cell::new(""),
        ];
    }
    let rewritten = answer
        .as_ref()
        .is_some_and(|mounted| mounted.fstype != OPEN && mounted.fstype != "unformatted");
    let filesystem = match &answer {
        Some(mounted) if mounted.fstype == OPEN => common::ui::Cell::set(copy::LUKS_OPEN),
        Some(mounted) if mounted.fstype != "unformatted" => common::ui::Cell::set(&mounted.fstype),
        _ if partition.fstype == "crypto_LUKS" => common::ui::Cell::new(copy::LUKS_CLOSED),
        _ => common::ui::Cell::new(&partition.fstype),
    };
    let format = match rewritten {
        true => common::ui::Cell::set(copy::FORMAT_TICK),
        false => common::ui::Cell::new(""),
    };
    let kind = match answer
        .as_ref()
        .is_some_and(|mounted| mounted.target == "/boot/efi")
    {
        true => common::ui::Cell::set(copy::EFI_CELL),
        false => common::ui::Cell::new(partition_type_said(&partition.parttype)),
    };
    let mount = match &answer {
        Some(mounted) => common::ui::Cell::set(&mounted.target),
        None => common::ui::Cell::new(""),
    };
    let label = renamed.unwrap_or(&partition.label);
    let said = match label.is_empty() {
        true => partition.device.clone(),
        false => format!("{} ({})", label, partition.device),
    };
    let answered = answer.is_some() || renamed.is_some();
    let device = match answered {
        true => common::ui::Cell::set(&format!("{branch}{said}")),
        false => common::ui::Cell::new(&format!("{branch}{said}")),
    };
    vec![
        device,
        common::ui::Cell::new(&copy::size_said(&partition.size)),
        filesystem,
        format,
        kind,
        mount,
    ]
}

pub(crate) fn partition_menu(
    partition: &Partition,
    held: Option<&CustomLayout>,
    boot: bool,
) -> (Vec<common::ui::MenuItem>, Vec<PartAction>) {
    if held.is_some_and(|layout| layout.deletes.contains(&partition.device)) {
        return (
            vec![common::ui::MenuItem::new(copy::RESET_CHANGES)],
            vec![PartAction::Reset],
        );
    }
    let answer = held.and_then(|layout| layout.answer(partition));
    let renamed = held.is_some_and(|layout| layout.renamed(&partition.device).is_some());
    let opened = held.is_some_and(|layout| {
        layout
            .opens
            .iter()
            .any(|open| open.partition == partition.device)
    });
    let mut items = Vec::new();
    let mut actions = Vec::new();
    let container = match &answer {
        Some(mounted) if mounted.fstype == OPEN => true,
        Some(mounted) if mounted.fstype != "unformatted" => false,
        _ => partition.fstype == "crypto_LUKS",
    };
    if container {
        if opened {
            let points = assign_children(&open_points());
            items.push(common::ui::MenuItem::under(copy::ASSIGN, &points));
            actions.push(PartAction::Assign);
        }
        items.push(common::ui::MenuItem::new(copy::FORMAT_ROW));
        actions.push(PartAction::Format);
        match opened {
            true => {
                items.push(common::ui::MenuItem::new(copy::CLOSE_ENCRYPTED));
                actions.push(PartAction::Close);
            }
            false => {
                items.push(common::ui::MenuItem::new(copy::OPEN_ENCRYPTED));
                actions.push(PartAction::Open);
            }
        }
    } else {
        let points = assigns(partition, answer.as_ref(), boot);
        if !points.is_empty() {
            let points = assign_children(&points);
            items.push(common::ui::MenuItem::under(copy::ASSIGN, &points));
            actions.push(PartAction::Assign);
        }
        items.push(common::ui::MenuItem::new(copy::FORMAT_ROW));
        actions.push(PartAction::Format);
    }
    items.push(common::ui::MenuItem::new(copy::RENAME_PART));
    actions.push(PartAction::Rename);
    if answer.is_some() || renamed {
        items.push(common::ui::MenuItem::new(copy::RESET_CHANGES));
        actions.push(PartAction::Reset);
    }
    items.push(common::ui::MenuItem::new(copy::DELETE_PART));
    actions.push(PartAction::Delete);
    (items, actions)
}

fn created_menu(create: &Created, boot: bool) -> (Vec<common::ui::MenuItem>, Vec<PartAction>) {
    let points: Vec<&'static str> = copy::mount_points()
        .into_iter()
        .filter(|target| *target != "/swap")
        .filter(|target| *target != "/boot" || boot)
        .filter(|target| create.fstype.is_empty() || fits(target, &create.fstype))
        .collect();
    let mut items = Vec::new();
    let mut actions = Vec::new();
    if !points.is_empty() {
        let points = assign_children(&points);
        items.push(common::ui::MenuItem::under(copy::ASSIGN, &points));
        actions.push(PartAction::Assign);
    }
    items.push(common::ui::MenuItem::new(copy::FORMAT_ROW));
    actions.push(PartAction::Format);
    items.push(common::ui::MenuItem::new(copy::RENAME_PART));
    actions.push(PartAction::Rename);
    items.push(common::ui::MenuItem::new(copy::DELETE_PART));
    actions.push(PartAction::Delete);
    (items, actions)
}

pub(crate) fn disk_menu(parts: usize) -> (Vec<common::ui::MenuItem>, Vec<PartAction>) {
    let mut items = vec![common::ui::MenuItem::new(copy::CREATE_PART)];
    let mut actions = vec![PartAction::Create];
    if parts > 0 {
        items.push(common::ui::MenuItem::new(copy::CLEAR_PARTS));
        actions.push(PartAction::Clear);
    }
    (items, actions)
}

fn created_cells(create: &Created, device: &str, branch: &str) -> Vec<common::ui::Cell> {
    let filesystem = match create.fstype.is_empty() {
        true => common::ui::Cell::new(""),
        false => common::ui::Cell::set(&create.fstype),
    };
    let kind = match create.target == "/boot/efi" {
        true => common::ui::Cell::set(copy::EFI_CELL),
        false => common::ui::Cell::new(""),
    };
    let mount = match create.target.is_empty() {
        true => common::ui::Cell::new(""),
        false => common::ui::Cell::set(&create.target),
    };
    let said = match create.label.is_empty() {
        true => device.to_string(),
        false => format!("{} ({device})", create.label),
    };
    vec![
        common::ui::Cell::set(&format!("{branch}{said}")),
        common::ui::Cell::new(&copy::size_said(&create.gb.to_string())),
        filesystem,
        common::ui::Cell::set(copy::CELL_NEW),
        kind,
        mount,
    ]
}

pub(crate) fn clamp_row(selectable: &[bool], at: usize) -> usize {
    let at = at.min(selectable.len().saturating_sub(1));
    if selectable.get(at).copied().unwrap_or(false) {
        return at;
    }
    if let Some(next) = (at + 1..selectable.len()).find(|next| selectable[*next]) {
        return next;
    }
    selectable[..at].iter().rposition(|ok| *ok).unwrap_or(0)
}

fn partition_type_said(parttype: &str) -> &'static str {
    match parttype.to_ascii_lowercase().as_str() {
        "c12a7328-f81f-11d2-ba4b-00a0c93ec93b" | "0xef" => "efi",
        "0fc63daf-8483-4772-8e79-3d69d8477de4" | "0x83" => "linux",
        "e6d6d379-f507-44c2-a23c-238f2a3df928" | "0x8e" => "lvm",
        "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f" | "0x82" => "swap",
        _ => "",
    }
}

pub(crate) fn usable_key(key: &Key, encrypted_root: bool) -> bool {
    encrypted_root || matches!(key, Key::Passphrase(_))
}

pub(crate) const LINUX_FILESYSTEMS: [&str; 4] = ["ext3", "ext4", "xfs", "btrfs"];

pub(crate) fn open_key(
    partition: &str,
    held: Option<&CustomLayout>,
    found: &Discovered,
) -> Result<Key, String> {
    let encrypted_root =
        held.is_some_and(|layout| layout.opens.iter().any(|open| open.target == "/"));
    match found
        .key(partition)
        .cloned()
        .filter(|key| usable_key(key, encrypted_root))
    {
        Some(key) => Ok(key),
        None => ask_key(!encrypted_root),
    }
}

fn ask_key(plain_root: bool) -> Result<Key, String> {
    let mut fields = vec![
        common::ui::Field::pick(copy::KEY_TYPE, key_methods(plain_root), Some(0)),
        common::ui::Field::secret(copy::KEY_PASSPHRASE, ""),
        common::ui::Field::text(copy::KEY_FILE_PATH, ""),
    ];
    let actions = [copy::KEY_OPEN, copy::KEY_BACK];
    let mut left = Vec::new();
    let filled = common::ui::in_overlay(40, 10, || {
        common::ui::form(
            &mut fields,
            &actions,
            |fields| key_short_of(&fields[0].value(), &fields[1].value(), &fields[2].value()),
            |_| Vec::new(),
            |fields| key_asked(&fields[0].value()),
            copy::INSTALL_KEYS,
            &[],
            &[],
            0,
            &mut left,
        )
    })?;
    match filled {
        common::ui::Filled::Took(0) => Ok(key_of(
            &fields[0].value(),
            &fields[1].value(),
            &fields[2].value(),
        )),
        _ => Err(common::ui::INTERRUPTED.to_string()),
    }
}

pub(crate) fn confirm_disk(disk: &DiskScan) -> Result<bool, String> {
    let (size, model) = match disk.detail.split_once("  ") {
        Some((size, rest)) => (
            size.to_string(),
            rest.split("  ").next().unwrap_or("").to_string(),
        ),
        None => (disk.detail.clone(), String::new()),
    };
    let heading = match model.is_empty() {
        true => format!("{}  {size}", disk.device),
        false => format!("{model} ({})  {size}", disk.device),
    };
    let rows: Vec<(String, String)> = match disk.partitions.is_empty() {
        true => vec![("no partitions".to_string(), String::new())],
        false => disk
            .partitions
            .iter()
            .map(|part| {
                let name = match part.label.is_empty() {
                    true => part.device.clone(),
                    false => format!("{} ({})", part.label, part.device),
                };
                let detail = match part.fstype.is_empty() {
                    true => copy::size_said(&part.size),
                    false => format!("{}  {}", copy::size_said(&part.size), part.fstype),
                };
                (name, detail)
            })
            .collect(),
    };
    common::ui::confirm_over(
        copy::USE_DISK,
        &heading,
        &rows,
        copy::USE_DISK_ACTION,
        copy::GO_BACK,
        common::ui::SIDE,
    )
}

pub(crate) fn container_number(payload: &Payload) -> usize {
    let plan = automatic_rows(payload, "", None, false, false);
    1 + plan
        .iter()
        .position(|(.., mount)| mount.as_str() == "/")
        .unwrap_or(0)
}

pub(crate) fn luks_window_rows(chosen: &str, luks_initramfs: bool) -> Vec<usize> {
    match () {
        _ if !luks_initramfs => vec![0],
        _ if Encryption::wants_passphrase(chosen) => vec![0, 1, 2, 3],
        _ if Encryption::wants_pin(chosen) => vec![0, 1, 4, 5],
        _ => vec![0],
    }
}

pub(crate) fn luks_window_start(chosen: &str) -> Option<usize> {
    match () {
        _ if Encryption::wants_passphrase(chosen) => Some(2),
        _ if Encryption::wants_pin(chosen) => Some(4),
        _ => None,
    }
}

pub(crate) fn edit_luks(
    kind: &str,
    passphrase: &str,
    partition: Option<&str>,
    tpm: bool,
    luks_initramfs: bool,
    with_none: bool,
) -> Result<Option<(String, String, String)>, String> {
    let options = kinds(tpm, luks_initramfs, with_none);
    let at = kind_at(&options, kind);
    let mut fields = vec![
        common::ui::Field::radio(copy::ENCRYPTION_TYPE, options, at),
        common::ui::Field::gap(),
        common::ui::Field::secret(copy::ROW_PASSPHRASE, passphrase),
        common::ui::Field::secret(copy::CONFIRM_PASSPHRASE, ""),
        common::ui::Field::secret(copy::ROW_PIN, ""),
        common::ui::Field::secret(copy::CONFIRM_PIN, ""),
    ];
    let header: Vec<common::ui::HeaderLine> = match partition {
        Some(partition) => vec![common::ui::HeaderLine::Row(format!(
            "{}: {partition}",
            copy::LUKS_PARTITION
        ))],
        None => Vec::new(),
    };
    let mut start = 0;
    let mut left: Vec<usize> = Vec::new();
    loop {
        let filled = common::ui::in_titled_overlay(
            common::ui::WINDOW_WIDTH,
            18,
            copy::PARTITION_ENCRYPTION,
            || {
                common::ui::form(
                    &mut fields,
                    &[],
                    |fields| {
                        luks_short_of(
                            written(&fields[0].value()),
                            &fields[2].value(),
                            &fields[3].value(),
                            &fields[4].value(),
                            &fields[5].value(),
                            luks_initramfs,
                        )
                    },
                    |fields| {
                        [pair_mismatch(fields, [2, 3]), pair_mismatch(fields, [4, 5])].concat()
                    },
                    |fields| luks_window_rows(written(&fields[0].value()), luks_initramfs),
                    copy::SUBMIT_KEYS,
                    &header,
                    &[],
                    start,
                    &mut left,
                )
            },
        )?;
        let chosen = written(&fields[0].value()).to_string();
        match filled {
            common::ui::Filled::Changed(0) => match luks_window_start(&chosen) {
                Some(at) => start = at,
                None => return Ok(Some((chosen, String::new(), String::new()))),
            },
            common::ui::Filled::Took(_) => {
                return Ok(Some((
                    chosen.clone(),
                    match Encryption::wants_passphrase(&chosen) {
                        true => fields[2].value(),
                        false => String::new(),
                    },
                    match Encryption::wants_pin(&chosen) {
                        true => fields[4].value(),
                        false => String::new(),
                    },
                )));
            }
            _ => return Ok(None),
        }
    }
}

pub(crate) fn slot_names(disk: &DiskScan, held: Option<&CustomLayout>) -> Vec<(usize, String)> {
    let short = |device: &str, label: &str| match label.is_empty() {
        true => device.rsplit('/').next().unwrap_or(device).to_string(),
        false => label.to_string(),
    };
    let mut names: Vec<(usize, String)> = disk
        .partitions
        .iter()
        .filter(|partition| !held.is_some_and(|held| held.deletes.contains(&partition.device)))
        .filter_map(|partition| {
            let label = held
                .and_then(|held| held.renamed(&partition.device))
                .unwrap_or(&partition.label);
            let number = partition_number(&partition.device).ok()?;
            Some((number, short(&partition.device, label)))
        })
        .collect();
    if let Some(held) = held {
        let devices = created_devices(
            &disk.device,
            &drawn_slots(&disk.partitions, &held.deletes),
            held.creates.len(),
        );
        for (device, create) in devices.iter().zip(&held.creates) {
            if let Ok(number) = partition_number(device) {
                names.push((number, short(device, &create.label)));
            }
        }
    }
    names
}

pub(crate) fn ask_create(
    rooms: Rooms,
    holes: Vec<common::ui::Hole>,
) -> Result<Option<(u64, u64)>, String> {
    let header = [common::ui::HeaderLine::Placement {
        holes,
        offset: 0,
        size: 1,
        available: copy::ROW_AVAILABLE.to_string(),
    }];
    let mut offset = "0".to_string();
    let mut size = match rooms.first {
        0 => String::new(),
        gb => gb.to_string(),
    };
    let mut refusal = String::new();
    loop {
        let label = match refusal.is_empty() {
            true => copy::ROW_PARTITION_SIZE.to_string(),
            false => refusal.clone(),
        };
        let mut fields = vec![
            common::ui::Field::measure(copy::ROW_OFFSET, &offset, "GB"),
            common::ui::Field::measure(&label, &size, "GB"),
        ];
        let mut left: Vec<usize> = Vec::new();
        let filled = common::ui::in_titled_overlay(
            common::ui::WINDOW_WIDTH,
            13,
            copy::NEW_PARTITION,
            || {
                common::ui::form(
                    &mut fields,
                    &[],
                    |fields| create_short_of(&fields[0].value(), &fields[1].value(), rooms.largest),
                    |_| Vec::new(),
                    |_| vec![0, 1],
                    copy::SUBMIT_KEYS,
                    &header,
                    &[],
                    1,
                    &mut left,
                )
            },
        )?;
        if !submitted(filled) {
            return Ok(None);
        }
        offset = fields[0].value();
        size = fields[1].value();
        match create_short_of(&offset, &size, rooms.largest) {
            None => {
                return Ok(Some((
                    size_gb(&offset).unwrap_or(0),
                    size_gb(&size).unwrap_or(0),
                )))
            }
            Some(said) => {
                refusal = match said.is_empty() {
                    true => copy::NEW_SIZE_NEEDED.to_string(),
                    false => said,
                }
            }
        }
    }
}

pub(crate) fn submitted(filled: common::ui::Filled) -> bool {
    matches!(
        filled,
        common::ui::Filled::Took(_) | common::ui::Filled::Changed(_)
    )
}

pub(crate) fn create_short_of(offset: &str, size: &str, room: u64) -> Option<String> {
    let Some(size) = size_gb(size) else {
        return Some(String::new());
    };
    let offset = size_gb(offset).unwrap_or(0);
    match size == 0 || offset.saturating_add(size) > room {
        true => Some(copy::custom_too_big(room)),
        false => None,
    }
}

pub(crate) fn ask_rename(partition: &str, held: &str) -> Result<Option<String>, String> {
    let mut fields = vec![common::ui::Field::text(copy::NEW_NAME, held)];
    let actions = [copy::RENAME_PART, copy::GO_BACK];
    let header: Vec<common::ui::HeaderLine> = match partition.is_empty() {
        true => Vec::new(),
        false => vec![common::ui::HeaderLine::Row(partition.to_string())],
    };
    let mut left: Vec<usize> = Vec::new();
    let filled = common::ui::in_overlay(40, 8, || {
        common::ui::form(
            &mut fields,
            &actions,
            |fields| match fields[0].value().is_empty() {
                true => Some(copy::NAME_NEEDED.to_string()),
                false => None,
            },
            |_| Vec::new(),
            |_| vec![0],
            copy::INSTALL_KEYS,
            &header,
            &[],
            0,
            &mut left,
        )
    })?;
    match filled {
        common::ui::Filled::Took(0) => Ok(Some(fields[0].value())),
        _ => Ok(None),
    }
}

pub(crate) fn key_methods(plain_root: bool) -> Vec<Choice> {
    let mut methods = vec![Choice::new(copy::KEY_PASSPHRASE, "")];
    if !plain_root {
        methods.push(Choice::new(copy::KEY_FILE, ""));
    }
    methods
}

fn key_asked(method: &str) -> Vec<usize> {
    match method == copy::KEY_FILE {
        true => vec![0, 2],
        false => vec![0, 1],
    }
}

pub(crate) fn luks_short_of(
    kind: &str,
    passphrase: &str,
    confirm: &str,
    pin: &str,
    confirm_pin: &str,
    luks_initramfs: bool,
) -> Option<String> {
    if kind != NONE && !luks_initramfs {
        return Some(copy::NO_LUKS_INITRAMFS.to_string());
    }
    if Encryption::wants_passphrase(kind) {
        if passphrase.is_empty() {
            return Some(copy::still_needs(&[copy::ROW_PASSPHRASE]));
        }
        if confirm.is_empty() {
            return Some(copy::still_needs(&[copy::CONFIRM_PASSPHRASE]));
        }
        return match passphrase == confirm {
            true => None,
            false => Some(String::new()),
        };
    }
    if Encryption::wants_pin(kind) {
        if pin.is_empty() {
            return Some(copy::still_needs(&[copy::ROW_PIN]));
        }
        if confirm_pin.is_empty() {
            return Some(copy::still_needs(&[copy::CONFIRM_PIN]));
        }
        return match pin == confirm_pin {
            true => None,
            false => Some(String::new()),
        };
    }
    None
}

fn key_short_of(method: &str, passphrase: &str, path: &str) -> Option<String> {
    match method == copy::KEY_FILE {
        true if path.is_empty() => Some(String::new()),
        true if !Path::new(path).is_file() => Some(copy::KEY_FILE_MISSING.to_string()),
        true => None,
        false if passphrase.is_empty() => Some(String::new()),
        false => None,
    }
}

fn key_of(method: &str, passphrase: &str, path: &str) -> Key {
    match method == copy::KEY_FILE {
        true => Key::File(PathBuf::from(path)),
        false => Key::Passphrase(passphrase.to_string()),
    }
}

pub(crate) fn layout_short_of(
    layout: &CustomLayout,
    seals: bool,
    table: Option<&DiskTable>,
    reserve: u64,
) -> Option<String> {
    if let Some(root) = layout
        .creates
        .iter()
        .find(|create| create.target == "/" && create.gb < reserve)
    {
        return Some(copy::custom_root_too_small(root.gb, reserve));
    }
    if let Some(table) = table {
        if let Some(why) =
            table.wrong_label_for(&layout.deletes, layout.creates.len(), layout.renames.len())
        {
            return Some(why);
        }
        if let Err(room) = place_creates(table, &layout.deletes, &layout.creates) {
            return Some(copy::custom_too_big(room));
        }
    }
    if layout.creates.iter().any(|create| create.fstype.is_empty()) {
        return Some(copy::CUSTOM_UNFORMATTED.to_string());
    }
    let targets: Vec<String> = layout
        .mounts
        .iter()
        .map(|mount| mount.target.clone())
        .chain(layout.opens.iter().map(|open| open.target.clone()))
        .chain(layout.creates.iter().map(|create| create.target.clone()))
        .collect();
    if targets.iter().any(String::is_empty) {
        return Some(copy::CUSTOM_UNMOUNTED.to_string());
    }
    if !targets.iter().any(|target| target == "/") {
        return Some(copy::CUSTOM_ROOT.to_string());
    }
    for (at, target) in targets.iter().enumerate() {
        if targets[..at].contains(target) {
            return Some(copy::CUSTOM_DUPLICATE.to_string());
        }
    }
    if !targets.iter().any(|target| target == "/boot/efi") {
        return Some(copy::CUSTOM_ESP.to_string());
    }
    if layout.mounts.iter().any(|mount| mount.fstype == "luks") {
        return Some(copy::CUSTOM_LUKS_LATER.to_string());
    }
    if seals {
        let formatted = layout
            .mounts
            .iter()
            .filter(|mount| mount.fstype != "unformatted")
            .map(|mount| (mount.target.as_str(), mount.fstype.as_str()))
            .chain(
                layout
                    .creates
                    .iter()
                    .map(|create| (create.target.as_str(), create.fstype.as_str())),
            );
        for (target, fstype) in formatted {
            if target == "/" && !["ext4", "btrfs"].contains(&fstype) {
                return Some(copy::CUSTOM_VERITY.to_string());
            }
        }
    }
    None
}
