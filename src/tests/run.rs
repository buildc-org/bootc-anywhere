use super::*;

#[test]
fn the_bootc_call_mounts_the_store_and_passes_every_recipe_flag() {
    let root = scratch("bootc-argv");
    let path = root.join(RECIPE);
    let raw = EMITTED.replace("\"bootloader\": \"grub2\"", "\"bootloader\": \"systemd\"");
    std::fs::write(&path, raw).expect("a recipe");
    let recipe = InstallRecipe::read(&path).expect("the emitted recipe is supported");
    let command = bootc_command(&recipe, Some("rd.luks.name=abcd=buildc-1"));
    let args: Vec<String> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();

    assert_eq!(command.get_program(), "podman");
    assert_eq!(
        args,
        [
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
            "-v",
            "/var/lib/buildc/store:/var/lib/buildc/store:ro",
            "-v",
            "/etc/containers/storage.conf:/etc/containers/storage.conf:ro",
            "-v",
            "/run/buildc-sysroot:/target",
            "ghcr.io/buildc-org/deb2:latest",
            "bootc",
            "install",
            "to-filesystem",
            "--skip-finalize",
            "--target-imgref",
            "ghcr.io/buildc-org/deb2:latest",
            "--composefs-backend",
            "--bootloader",
            "systemd",
            "--generic-image",
            "--karg",
            "rd.luks.name=abcd=buildc-1",
            "/target",
        ]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_last_screen_heads_the_key_and_the_steps() {
    let volumes = ["encrypted disk: /dev/vda".to_string()];
    let rows = done_rows(
        Some("cafebabe"),
        None,
        Some(copy::next_steps_setup()),
        &[],
        &volumes,
        &Offered::None,
    );
    let said: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(said[0], copy::RECOVERY_HEADING);
    assert_eq!(said[1], copy::write_down());
    assert_eq!(said[2], "");
    assert_eq!(said[3], "encrypted disk: /dev/vda");
    assert_eq!(said[4], "");
    assert_eq!(said[5], "cafebabe");
    assert_eq!(said[6], "");
    assert_eq!(said[7], copy::NEXT_STEPS_HEADING);
    assert_eq!(said[said.len() - 1], copy::logging(None));
    assert!(
        rows[0].heading && rows[7].heading,
        "the headings are headings"
    );
    assert!(
        !rows[1].heading && !rows[3].heading && !rows[5].heading,
        "the content is content"
    );
    let wrapped: Vec<&str> = said[8..said.len() - 1].to_vec();
    assert_eq!(wrapped.join(" "), copy::next_steps_setup());
    assert!(
        wrapped
            .iter()
            .all(|line| line.chars().count() <= common::ui::ROW_ROOM),
        "{wrapped:?}"
    );
}

fn a_ready() -> Ready {
    Ready {
        image: "example.invalid/image:1".to_string(),
        disk: "/dev/vda".to_string(),
        partition: "/dev/vda2".to_string(),
        key: Key::Passphrase("recovery".to_string()),
        pin: false,
    }
}

#[test]
fn a_failed_finalize_draws_the_screen_again_with_the_reason() {
    let mut offered = Offered::Ready(a_ready());
    let mut drawn = Vec::new();
    let mut restarts = 0;
    choosing(
        &mut offered,
        |offered| {
            drawn.push(match offered {
                Offered::Ready(_) => 0,
                Offered::Unavailable(_) => 1,
                Offered::None => 2,
            });
            Ok(Some(drawn.len() - 1))
        },
        |_| Err("the seal failed".to_string()),
        || {
            restarts += 1;
            Ok(())
        },
    )
    .expect("the manual action ends the loop");
    assert_eq!(drawn, [0, 1], "the screen is drawn again after the failure");
    assert_eq!(restarts, 1);
    match &offered {
        Offered::Unavailable(why) => assert_eq!(why, copy::AUTO_FAILED),
        _ => panic!("a failed finalize must leave the automatic action unavailable"),
    }
}

#[test]
fn a_successful_finalize_restarts_at_once() {
    let mut offered = Offered::Ready(a_ready());
    let mut drawn = 0;
    let mut finalized = 0;
    let mut restarts = 0;
    choosing(
        &mut offered,
        |_| {
            drawn += 1;
            Ok(Some(0))
        },
        |_| {
            finalized += 1;
            Ok(())
        },
        || {
            restarts += 1;
            Ok(())
        },
    )
    .expect("the restart ends the loop");
    assert_eq!((drawn, finalized, restarts), (1, 1, 1));
}

#[test]
fn esc_ends_the_last_screen_without_restarting() {
    let mut offered = Offered::Ready(a_ready());
    choosing(
        &mut offered,
        |_| Ok(None),
        |_| panic!("no action was taken"),
        || panic!("nothing asked for a restart"),
    )
    .expect("esc is an ending");
    assert!(matches!(offered, Offered::Ready(_)));
}

#[test]
fn the_last_screen_explains_the_two_actions() {
    let ready = |pin| {
        Offered::Ready(Ready {
            image: "example.invalid/image:1".to_string(),
            disk: "/dev/vda".to_string(),
            partition: "/dev/vda2".to_string(),
            key: Key::Passphrase("recovery".to_string()),
            pin,
        })
    };
    let rows = done_rows(None, None, None, &[], &[], &ready(true));
    let said: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
    let window = said.len() - 1;
    let explained = &said[..window];
    let start = explained
        .iter()
        .position(|line| line.starts_with("The reboot unlocks"))
        .expect("the explanation is drawn");
    let paragraphs = copy::automatic_explanation();
    let joined = explained[start..]
        .iter()
        .filter(|line| !line.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        joined,
        paragraphs.join(" "),
        "every wrapped line is part of the one explanation"
    );
    assert!(
        explained[start..]
            .iter()
            .all(|line| line.chars().count() <= common::ui::ROW_ROOM),
        "{explained:?}"
    );
    let second = explained
        .iter()
        .position(|line| line.starts_with("The manual restart"))
        .expect("the manual paragraph is drawn");
    assert!(explained[second - 1].is_empty(), "a blank parts them");
    assert_eq!(said[window], copy::AUTO_WINDOW_PIN);
    assert!(rows[window].warning, "the window is the must-read row");
    let plain = done_rows(None, None, None, &[], &[], &ready(false));
    assert_eq!(
        plain.last().map(|row| row.label.as_str()),
        Some(copy::AUTO_WINDOW)
    );
    let refused = done_rows(
        None,
        None,
        None,
        &[],
        &[],
        &Offered::Unavailable(copy::AUTO_NO_POLICY.to_string()),
    );
    assert!(
        refused.last().is_some_and(|row| !row.warning),
        "an unavailable action draws no window"
    );
}

#[test]
#[ignore]
fn installs_from_the_payload_the_parent_staged() {
    let root = PathBuf::from(
        std::env::var("BUILDC_TEST_PAYLOAD")
            .expect("the payload is staged only by its parent test"),
    );
    let Ok(Found::Image(payload)) = classify(&root) else {
        panic!("the parent stages a payload");
    };
    let mut answers = answered(NONE, "");
    answers.disk = "/dev/buildc-test-absent".to_string();
    let refused = run(&payload, &mut answers, &Prompt::silent()).expect_err("an altered image");
    std::fs::write(root.join("refused"), refused).expect("the child reports the refusal");
}

#[test]
#[ignore]
fn an_install_from_an_altered_image_is_refused_before_the_disk() {
    let (root, conf) = imported_store("run-guard");
    let inspected = in_store(inspect(STORED, "{{.Id}}"), &conf);
    std::fs::write(root.join(IMAGE_ID), &inspected.stdout).expect("an image-id");
    let graph = root.join("graph");
    std::fs::write(
        root.join(RECIPE),
        format!(
            "{{\"image\": \"{STORED}\", \"hostname\": \"host\", \"filesystem\": \"ext4\", \
             \"bootloader\": \"grub2\", \"additionalImageStores\": [\"{}\"]}}",
            graph.display()
        ),
    )
    .expect("a recipe");
    alter_layer(&root);

    let child = Command::new(std::env::current_exe().expect("the test binary"))
        .args([
            "--ignored",
            "--exact",
            "tests::run::installs_from_the_payload_the_parent_staged",
        ])
        .env("CONTAINERS_STORAGE_CONF", &conf)
        .env("BUILDC_TEST_PAYLOAD", &root)
        .output()
        .expect("the test binary re-runs itself as the install");
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stdout)
    );
    let refused = std::fs::read_to_string(root.join("refused")).expect("the child's refusal");
    assert!(refused.contains("does not read back"), "{refused}");
    remove_store(&root);
}
