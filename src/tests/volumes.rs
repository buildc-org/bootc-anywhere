use super::*;

#[test]
fn opening_a_container_names_cryptsetup_and_the_mapper() {
    let open = |key| LuksOpen {
        partition: "/dev/vda2".to_string(),
        target: "/".to_string(),
        key,
    };
    let args = |open: &LuksOpen| -> Vec<String> {
        let command = open_command(open, "tect-1");
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    };
    assert_eq!(
        args(&open(Key::File(PathBuf::from("/run/keyfile")))),
        [
            "-q",
            "luksOpen",
            "--key-file",
            "/run/keyfile",
            "/dev/vda2",
            "tect-1"
        ]
    );
    assert_eq!(
        args(&open(Key::Passphrase("opensesame".to_string()))),
        ["-q", "luksOpen", "--key-file", "-", "/dev/vda2", "tect-1"]
    );
    assert_eq!(
        args(&open(Key::Data(b"opensesame".to_vec()))),
        ["-q", "luksOpen", "--key-file", "-", "/dev/vda2", "tect-1"]
    );
}
