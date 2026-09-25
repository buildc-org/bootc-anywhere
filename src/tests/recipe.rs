use super::*;

#[test]
fn the_completed_recipe_keeps_every_derived_field_and_hashes_the_password() {
    let root = scratch("complete");
    let recipe = root.join(RECIPE);
    std::fs::write(&recipe, EMITTED).expect("a recipe");
    let answers = Answers {
        disk: "/dev/vda".to_string(),
        hostname: "deb2".to_string(),
        user: "tect".to_string(),
        password: "hunter2".to_string(),
        opened: Opened::Keep,
        encryption: Encryption {
            kind: "luks-passphrase".to_string(),
            passphrase: "opensesame".to_string(),
            pin: String::new(),
        },
        data: Data::default(),
        layout: None,
    };
    let done = complete(&recipe, &answers).expect("the person's half goes in");

    for (key, value) in [
        ("image", "\"ghcr.io/buildc-org/deb2:latest\""),
        ("composeFsBackend", "true"),
        ("bootloader", "\"grub2\""),
        ("filesystem", "\"ext4\""),
        ("hostname", "\"deb2\""),
        ("disk", "\"/dev/vda\""),
    ] {
        let held = json::field(&done, key).map(|v| v.render().trim().to_string());
        assert_eq!(held.as_deref(), Some(value), "{key}");
    }

    let user = json::field(&done, "user").expect("an account");
    assert_eq!(json::strings(user, "groups"), ["sudo"]);
    assert_eq!(json::text(user, "username").as_deref(), Some("tect"));
    let hash = json::text(user, "password").expect("a password");
    assert!(hash.starts_with("$6$"), "{hash}");
    assert!(!hash.contains("hunter2"), "{hash}");

    let encryption = json::field(&done, "encryption").expect("an encryption");
    assert_eq!(
        json::text(encryption, "type").as_deref(),
        Some("luks-passphrase")
    );
    assert_eq!(
        json::text(encryption, "passphrase").as_deref(),
        Some("opensesame")
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_assign_list_follows_the_filesystem() {
    let partition = |fstype: &str| Partition {
        device: "/dev/vda1".to_string(),
        size: "512M".to_string(),
        fstype: fstype.to_string(),
        label: String::new(),
        parttype: String::new(),
        uuid: String::new(),
    };
    assert_eq!(
        assigns(&partition("vfat"), None, true, false),
        ["/boot/efi"]
    );
    assert_eq!(
        assigns(&partition("ext4"), None, true, false),
        ["/", "/boot", "/var", "/var/home"]
    );
    assert_eq!(
        assigns(&partition("ext4"), None, false, false),
        ["/", "/var", "/var/home"]
    );
    assert_eq!(assigns(&partition("swap"), None, true, false), ["/swap"]);
    assert!(assigns(&partition(""), None, true, false).is_empty());
    assert_eq!(
        assigns(&partition("ext4"), None, true, true),
        ["/", "/boot", "/var/home"]
    );
    let chosen = Mounted {
        target: String::new(),
        fstype: "fat32".to_string(),
    };
    assert_eq!(
        assigns(&partition(""), Some(&chosen), true, false),
        ["/boot/efi"]
    );
}

#[test]
fn the_staged_recipe_is_removed_when_its_guard_drops() {
    let recipe = Json::parse(r#"{"hostname":"host"}"#).expect("the test recipe parses");
    let path = {
        let staged = stage(&recipe).expect("the recipe stages");
        assert!(
            staged.0.exists(),
            "the recipe is readable while fisherman needs it"
        );
        staged.0.clone()
    };
    assert!(!path.exists(), "the guard removed the staged recipe");
}
