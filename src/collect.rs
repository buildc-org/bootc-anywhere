use super::*;

impl Answers {
    pub fn collect(
        payload: &Payload,
        given: Given,
        prompt: &Prompt,
    ) -> Result<Option<Self>, String> {
        let named = given.encryption.is_some();
        let seeded = Self::seeded(payload, given, prompt)?;
        if !prompt.draws() {
            return Ok(Some(seeded));
        }
        let found_disks = disks(&sys_block(), &in_use_now());
        let mut disk = seeded.disk.clone();
        if disk.is_empty() && found_disks.len() == 1 {
            disk = found_disks[0].0.clone();
        }
        let chosen = seeded.encryption.kind.clone();
        let mut kind = match (
            named,
            chosen.as_str(),
            tpm().exists() && payload.luks_initramfs,
        ) {
            (false, NONE, true) => "tpm2-luks".to_string(),
            _ => chosen,
        };
        let mut layout: Option<CustomLayout> = None;
        let mut fields = seeded.fields(payload, &disk, layout.as_ref(), "", &[]);
        let panel = panel(payload);
        let mut cursor: Option<usize> = None;
        let mut reopen_on_table = false;
        let mut found = Discovered::default();
        let mut found_disk = String::new();
        let mut start = 0;
        let mut left: Vec<usize> = Vec::new();
        loop {
            let manual = fields[ROW_LAYOUT].value() == copy::LAYOUT_MANUAL;
            layout = match (manual, layout.take()) {
                (true, Some(held)) if held.disk == disk => Some(held),
                (true, _) => Some(CustomLayout::empty(&disk)),
                (false, _) => None,
            };
            let partitions: Vec<(String, Vec<Partition>)> = match manual {
                true => found_disks
                    .iter()
                    .map(|(at, _)| Ok((at.clone(), partitions(at)?)))
                    .collect::<Result<_, String>>()?,
                false => Vec::new(),
            };
            let chosen = partitions
                .iter()
                .find(|(at, _)| at == &disk)
                .map(|(_, parts)| parts.as_slice())
                .unwrap_or(&[]);
            let table_now = match manual && !disk.is_empty() {
                true => Some(disk_table(&disk)?),
                false => None,
            };
            if manual && found_disk != disk {
                found = old_keys(&disk, chosen);
                found_disk = disk.clone();
            }
            if !manual {
                found = Discovered::default();
                found_disk.clear();
            }
            let separate = fields[ROW_DATA].value() == copy::DATA_SEPARATE;
            let size = match separate {
                true => fields[ROW_SIZE].value(),
                false => String::new(),
            };
            let table = layout_table(
                &found_disks,
                &disk,
                layout.as_ref(),
                &partitions,
                payload,
                &size,
                separate,
                kind != NONE,
            );
            let at = match cursor {
                Some(at) => at,
                None => table
                    .kinds
                    .iter()
                    .position(|kind| matches!(kind, RowKind::Disk(at) if at == &disk))
                    .unwrap_or(0),
            };
            fields[ROW_TABLE] = table.field(clamp_row(&table.selectable, at), !disk.is_empty());
            fields[ROW_ENCRYPTION] = match layout.as_ref().filter(|layout| !layout.opens.is_empty())
            {
                Some(layout) => encryption_row(
                    Some(layout),
                    &fields[ROW_ENCRYPTION],
                    tpm().exists(),
                    payload.luks_initramfs,
                ),
                None if manual => common::ui::Field::action(copy::ROW_ENCRYPTION, shown(&kind)),
                None => common::ui::Field::action(copy::ROW_ENCRYPTION, shown(&kind)),
            };
            if reopen_on_table {
                reopen_on_table = false;
                start = ROW_TABLE;
            }
            let actions = [
                copy::INSTALL,
                copy::EXIT_SHELL,
                copy::RESTART,
                copy::SHUT_DOWN,
            ];
            let filled = common::ui::form(
                &mut fields,
                &actions,
                |fields| {
                    short_of(
                        fields,
                        &disk,
                        layout.as_ref(),
                        payload,
                        tpm().exists(),
                        table_now.as_ref(),
                    )
                },
                |fields| pair_mismatch(fields, [ROW_PASSWORD, ROW_CONFIRM]),
                |fields| asked(fields),
                copy::INSTALL_KEYS,
                &panel,
                &[
                    (ROW_HOSTNAME, copy::SETUP_OS),
                    (ROW_LAYOUT, copy::SETUP_DISK),
                ],
                start,
                &mut left,
            );
            match filled {
                Ok(common::ui::Filled::Took(0)) => {
                    let mut answers = Self::of(&fields, disk.clone(), layout.clone());
                    if let Some(layout) = answers
                        .layout
                        .as_mut()
                        .filter(|layout| !layout.deletes.is_empty() || !layout.creates.is_empty())
                    {
                        let shown = table_now
                            .as_ref()
                            .ok_or("the partition table was not available to confirm")?;
                        let state = disk_state(&layout.disk)?;
                        if state.table != *shown {
                            return Err(copy::table_changed(&layout.disk));
                        }
                        layout.confirmed = Some(state);
                    }
                    let warning = erase_warning(&payload.boot, &firmware(payload));
                    if common::ui::decide(
                        copy::INSTALLATION_SUMMARY,
                        &match &answers.layout {
                            Some(layout) if !layout.deletes.is_empty() => {
                                copy::removing_partitions(&answers.disk, layout.deletes.len())
                            }
                            Some(_) => copy::changing_partitions(&answers.disk),
                            None => copy::erasing(&answers.disk),
                        },
                        &warning,
                        &answers.summary(payload),
                        copy::READY,
                        copy::START_INSTALLATION,
                        copy::GO_BACK,
                    )? {
                        return Ok(Some(answers));
                    }
                }
                Ok(common::ui::Filled::Took(1)) => {
                    let note = copy::switch_note(console_vt().unwrap_or(1));
                    if common::ui::decide(
                        copy::EXIT_SHELL,
                        &note,
                        &[],
                        &[],
                        copy::EXIT_SHELL,
                        copy::EXIT_SHELL,
                        copy::GO_BACK,
                    )? {
                        if !under_media_installer() {
                            return Ok(None);
                        }
                        switch_to_shell()?;
                    }
                }
                Ok(common::ui::Filled::Took(2)) => {
                    restart()?;
                    return Ok(None);
                }
                Ok(common::ui::Filled::Took(_)) => {
                    power_off()?;
                    return Ok(None);
                }
                Ok(common::ui::Filled::Table { row, item, child }) => {
                    cursor = Some(row);
                    reopen_on_table = true;
                    match table.kinds.get(row) {
                        Some(RowKind::Disk(at)) => {
                            let action = item
                                .and_then(|at| table.actions.get(row).and_then(|row| row.get(at)))
                                .copied();
                            match action {
                                Some(PartAction::Create) => {
                                    let planned: u64 = layout
                                        .as_ref()
                                        .map(|held| {
                                            held.creates.iter().map(|create| create.gb).sum()
                                        })
                                        .unwrap_or_default();
                                    let deletes = layout
                                        .as_ref()
                                        .map(|held| held.deletes.clone())
                                        .unwrap_or_default();
                                    let room = disk_table(at)?
                                        .appendable_gb(&deletes)
                                        .saturating_sub(planned);
                                    if let Some(gb) = ask_size(room)? {
                                        held_layout(&mut layout, &disk).creates.push(Created {
                                            gb,
                                            ..Default::default()
                                        });
                                    }
                                }
                                Some(PartAction::Clear) => {
                                    if common::ui::confirm(
                                        &copy::clear_all(chosen.len()),
                                        copy::USE_DISK_ACTION,
                                        copy::GO_BACK,
                                    )? {
                                        let held = held_layout(&mut layout, &disk);
                                        held.deletes =
                                            chosen.iter().map(|part| part.device.clone()).collect();
                                        held.mounts.clear();
                                        held.opens.clear();
                                    }
                                }
                                _ => {
                                    if manual || confirm_disk(at, &found_disks)? {
                                        disk = at.clone();
                                    }
                                }
                            }
                        }
                        Some(RowKind::Created { index }) => {
                            let index = *index;
                            let Some(action) = item
                                .and_then(|at| table.actions.get(row).and_then(|row| row.get(at)))
                                .copied()
                            else {
                                continue;
                            };
                            let children: &[String] = item
                                .and_then(|at| table.menus.get(row).and_then(|row| row.get(at)))
                                .map(|menu| menu.children.as_slice())
                                .unwrap_or(&[]);
                            match action {
                                PartAction::Assign => {
                                    if let Some(target) = child.and_then(|at| children.get(at)) {
                                        if let Some(create) = layout
                                            .as_mut()
                                            .and_then(|held| held.creates.get_mut(index))
                                        {
                                            create.target = match target.as_str() {
                                                copy::UNASSIGN => String::new(),
                                                target => target.to_string(),
                                            };
                                        }
                                    }
                                }
                                PartAction::Format => {
                                    let options = copy::plain_formats();
                                    let current = layout
                                        .as_ref()
                                        .and_then(|held| held.creates.get(index))
                                        .map(|create| create.fstype.clone())
                                        .unwrap_or_default();
                                    let at = options
                                        .iter()
                                        .position(|option| *option == current.as_str())
                                        .unwrap_or(0);
                                    if let Some(taken) = common::ui::choose(
                                        copy::SELECT_FORMAT,
                                        copy::CHOOSE_KEYS,
                                        &options,
                                        at,
                                    )? {
                                        if let Some(create) = layout
                                            .as_mut()
                                            .and_then(|held| held.creates.get_mut(index))
                                        {
                                            create.fstype = options[taken].to_string();
                                            if !create.target.is_empty()
                                                && !fits(&create.target, &create.fstype)
                                            {
                                                create.target.clear();
                                            }
                                        }
                                    }
                                }
                                PartAction::Delete => {
                                    if let Some(held) = layout.as_mut() {
                                        if index < held.creates.len() {
                                            held.creates.remove(index);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        Some(RowKind::Part { index }) => {
                            let Some(partition) = chosen.get(*index) else {
                                continue;
                            };
                            let Some(action) = item
                                .and_then(|at| table.actions.get(row).and_then(|row| row.get(at)))
                                .copied()
                            else {
                                continue;
                            };
                            let children: &[String] = item
                                .and_then(|at| table.menus.get(row).and_then(|row| row.get(at)))
                                .map(|menu| menu.children.as_slice())
                                .unwrap_or(&[]);
                            match action {
                                PartAction::Assign => {
                                    if let Some(target) = child.and_then(|at| children.get(at)) {
                                        match target.as_str() {
                                            copy::UNASSIGN => {
                                                place_unassign(&mut layout, partition)
                                            }
                                            _ => {
                                                place_target(&mut layout, &disk, partition, target)
                                            }
                                        }
                                    }
                                }
                                PartAction::Format => {
                                    let options = copy::format_options();
                                    let answer =
                                        layout.as_ref().and_then(|held| held.answer(partition));
                                    let current = effective_fs(partition, answer.as_ref());
                                    let at = options
                                        .iter()
                                        .position(|option| *option == current.as_str())
                                        .unwrap_or(0);
                                    if let Some(taken) = common::ui::choose(
                                        copy::SELECT_FORMAT,
                                        copy::CHOOSE_KEYS,
                                        &options,
                                        at,
                                    )? {
                                        match options[taken] {
                                            "luks" => {
                                                if let Some((_, passphrase, _)) = edit_luks(
                                                    KINDS[3].0,
                                                    "",
                                                    Some(&partition.device),
                                                    tpm().exists(),
                                                    payload.luks_initramfs,
                                                    false,
                                                )? {
                                                    place_format(
                                                        &mut layout,
                                                        &disk,
                                                        partition,
                                                        "luks",
                                                    );
                                                    if let Some(mount) =
                                                        layout.as_mut().and_then(|held| {
                                                            held.mounts.iter_mut().find(|mount| {
                                                                mount.partition == partition.device
                                                            })
                                                        })
                                                    {
                                                        mount.passphrase = passphrase;
                                                    }
                                                }
                                            }
                                            format => {
                                                place_format(&mut layout, &disk, partition, format)
                                            }
                                        }
                                    }
                                }
                                PartAction::Reset => {
                                    if let Some(layout) = layout.as_mut() {
                                        layout
                                            .mounts
                                            .retain(|mount| mount.partition != partition.device);
                                        layout
                                            .opens
                                            .retain(|open| open.partition != partition.device);
                                        layout.deletes.retain(|at| at != &partition.device);
                                    }
                                }
                                PartAction::Delete => {
                                    let device = partition.device.clone();
                                    let held = held_layout(&mut layout, &disk);
                                    held.mounts.retain(|mount| mount.partition != device);
                                    held.opens.retain(|open| open.partition != device);
                                    if !held.deletes.contains(&device) {
                                        held.deletes.push(device);
                                    }
                                }
                                PartAction::Open => {
                                    let key = match open_key(
                                        &partition.device,
                                        layout.as_ref(),
                                        &found,
                                    ) {
                                        Ok(key) => key,
                                        Err(err) if leaving(&err) => continue,
                                        Err(err) => return Err(err),
                                    };
                                    let layout = held_layout(&mut layout, &disk);
                                    layout
                                        .mounts
                                        .retain(|mount| mount.partition != partition.device);
                                    layout
                                        .opens
                                        .retain(|open| open.partition != partition.device);
                                    layout.opens.push(LuksOpen {
                                        partition: partition.device.clone(),
                                        target: String::new(),
                                        key,
                                    });
                                }
                                PartAction::Close => {
                                    if let Some(layout) = layout.as_mut() {
                                        layout
                                            .opens
                                            .retain(|open| open.partition != partition.device);
                                    }
                                }
                                PartAction::Clear | PartAction::Create => {}
                            }
                        }
                        _ => {}
                    }
                }
                Ok(common::ui::Filled::Changed(row)) => {
                    start = row;
                }
                Ok(common::ui::Filled::Opened(ROW_ENCRYPTION)) if layout.is_none() => {
                    let at = (!disk.is_empty())
                        .then(|| fdisk::partname(&disk, container_number(payload)));
                    if let Some((chosen, passphrase, pin)) = edit_luks(
                        &kind,
                        &fields[ROW_PASSPHRASE].value(),
                        at.as_deref(),
                        tpm().exists(),
                        payload.luks_initramfs,
                        true,
                    )? {
                        kind = chosen;
                        fields[ROW_PASSPHRASE] =
                            common::ui::Field::secret(copy::ROW_PASSPHRASE, &passphrase);
                        fields[ROW_PIN] = common::ui::Field::secret(copy::ROW_PIN, &pin);
                    }
                    start = ROW_ENCRYPTION;
                }
                Ok(common::ui::Filled::Opened(_)) => {}
                Ok(common::ui::Filled::Left) => match leave(prompt)? {
                    Leave::Shell => return Ok(None),
                    Leave::Over => {
                        disk.clear();
                        kind = NONE.to_string();
                        layout = None;
                        cursor = None;
                        start = 0;
                        left.clear();
                        fields = Self::seeded(payload, Given::default(), prompt)?.fields(
                            payload,
                            &disk,
                            None,
                            "",
                            &[],
                        )
                    }
                    Leave::Back => {}
                },
                Err(err) if leaving(&err) => match leave(prompt)? {
                    Leave::Shell => return Ok(None),
                    Leave::Over => {
                        disk.clear();
                        kind = NONE.to_string();
                        layout = None;
                        cursor = None;
                        start = 0;
                        left.clear();
                        fields = Self::seeded(payload, Given::default(), prompt)?.fields(
                            payload,
                            &disk,
                            None,
                            "",
                            &[],
                        )
                    }
                    Leave::Back => {}
                },
                Err(err) => return Err(err),
            }
        }
    }

    pub(crate) fn seeded(payload: &Payload, given: Given, prompt: &Prompt) -> Result<Self, String> {
        if !prompt.draws() {
            return Ok(Self {
                disk: ask_disk(given.disk, None, prompt)?,
                hostname: prompt.text(
                    given.hostname,
                    copy::INSTALL_NAME,
                    "--hostname",
                    Some(&payload.hostname),
                )?,
                user: prompt.text(given.user, copy::INSTALL_USER, "--user", None)?,
                password: prompt.text(
                    given.password,
                    copy::INSTALL_PASSWORD,
                    "--password",
                    None,
                )?,
                encryption: ask_encryption(
                    given.encryption,
                    given.passphrase,
                    given.pin,
                    prompt,
                    tpm().exists(),
                    payload.luks_initramfs,
                )?,
                data: Data::default(),
                layout: None,
                opened: Opened::Keep,
            });
        }
        Ok(Self {
            disk: given.disk.unwrap_or_default(),
            hostname: given
                .hostname
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| payload.hostname.clone()),
            user: given.user.unwrap_or_default(),
            password: given.password.unwrap_or_default(),
            encryption: Encryption {
                kind: match given.encryption {
                    Some(kind) => named(kind)?,
                    None => NONE.to_string(),
                },
                passphrase: given.passphrase.unwrap_or_default(),
                pin: given.pin.unwrap_or_default(),
            },
            data: Data::default(),
            layout: None,
            opened: Opened::Keep,
        })
    }

    pub(crate) fn fields(
        &self,
        payload: &Payload,
        disk: &str,
        layout: Option<&CustomLayout>,
        var_size: &str,
        partitions: &[(String, Vec<Partition>)],
    ) -> Vec<common::ui::Field> {
        use common::ui::Field;
        let found = disks(&sys_block(), &in_use_now());
        let table = layout_table(
            &found,
            disk,
            layout,
            partitions,
            payload,
            var_size,
            !self.data.size.is_empty(),
            self.encryption.kind != NONE,
        );
        vec![
            Field::text(copy::ROW_HOSTNAME, &self.hostname),
            Field::text(copy::ROW_ACCOUNT, &self.user),
            Field::secret(copy::ROW_PASSWORD, &self.password),
            Field::secret(copy::ROW_CONFIRM, &self.password),
            Field::pick_change(
                copy::ROW_LAYOUT,
                vec![
                    Choice::new(copy::LAYOUT_WHOLE, ""),
                    Choice::new(copy::LAYOUT_MANUAL, ""),
                ],
                Some(usize::from(layout.is_some())),
            ),
            encryption_row(
                layout,
                &Field::action(copy::ROW_ENCRYPTION, shown(&self.encryption.kind)),
                tpm().exists(),
                payload.luks_initramfs,
            ),
            Field::pick_change(copy::ROW_DATA, data_rows(), Some(0)),
            Field::measure(
                &format!("\u{2514}\u{2500} {}", copy::ROW_SIZE),
                &self.data.size,
                "GB",
            ),
            table.field(0, !disk.is_empty()),
            Field::secret(copy::ROW_PASSPHRASE, &self.encryption.passphrase),
            Field::secret(copy::ROW_PIN, &self.encryption.pin),
        ]
    }

    pub(crate) fn of(
        fields: &[common::ui::Field],
        disk: String,
        layout: Option<CustomLayout>,
    ) -> Self {
        let at = |row: usize| fields[row].value();
        let held = held_pick(fields, layout.as_ref(), &disk);
        let manual = held.is_some();
        let opens = matches!(fields[ROW_ENCRYPTION], common::ui::Field::Pick { .. });
        Self {
            disk,
            hostname: at(ROW_HOSTNAME),
            user: at(ROW_ACCOUNT),
            password: at(ROW_PASSWORD),
            encryption: match manual {
                true => Encryption {
                    kind: NONE.to_string(),
                    passphrase: String::new(),
                    pin: String::new(),
                },
                false => {
                    let kind = written(&at(ROW_ENCRYPTION));
                    Encryption {
                        kind: kind.to_string(),
                        passphrase: match kind == NONE {
                            true => String::new(),
                            false => at(ROW_PASSPHRASE),
                        },
                        pin: match kind == "tpm2-luks-pin" {
                            true => at(ROW_PIN),
                            false => String::new(),
                        },
                    }
                }
            },
            data: match manual {
                true => Data::default(),
                false => chose(&at(ROW_DATA), &at(ROW_SIZE)),
            },
            opened: match manual {
                true if opens => Opened::of(&at(ROW_ENCRYPTION)),
                _ => Opened::Keep,
            },
            layout: held,
        }
    }

    pub(crate) fn summary(&self, payload: &Payload) -> Vec<(String, String)> {
        let mut rows = vec![
            (copy::ROW_DISK.to_string(), self.disk.clone()),
            (copy::ROW_HOSTNAME.to_string(), self.hostname.clone()),
            (copy::ROW_ACCOUNT.to_string(), self.user.clone()),
            (
                copy::ROW_PASSWORD.to_string(),
                copy::PASSWORD_SET.to_string(),
            ),
            (
                copy::ROW_ENCRYPTION.to_string(),
                shown(&self.encryption.kind).to_string(),
            ),
        ];
        if self.layout.is_none() {
            rows.push((copy::ROW_DATA.to_string(), data_said(&self.data)));
        }
        match &self.layout {
            Some(layout) => {
                rows.extend(
                    layout.deletes.iter().map(|device| {
                        (copy::ROW_REMOVED.to_string(), copy::summary_deleted(device))
                    }),
                );
                rows.extend(layout.creates.iter().map(|create| {
                    (
                        create.target.clone(),
                        copy::summary_created(create.gb, &create.fstype),
                    )
                }));
                rows.extend(layout.mounts.iter().map(|mount| {
                    let how = match mount.fstype.as_str() {
                        "unformatted" => format!("{}  kept", mount.partition),
                        fstype => format!("{}  format as {fstype}", mount.partition),
                    };
                    (mount.target.clone(), how)
                }));
                rows.extend(layout.opens.iter().map(|open| {
                    (
                        open.target.clone(),
                        copy::opened(&open.partition, at_boot(open, self.opened)),
                    )
                }));
                if let Some(chain) = copy::boot_chain(&payload.boot) {
                    rows.push(("boot chain".to_string(), chain.to_string()));
                }
            }
            None => rows.extend(copy::written_over(
                &payload.bootloader,
                &payload.filesystem,
                &self.data.size,
                self.encryption.kind != NONE,
            )),
        }
        rows
    }
}
