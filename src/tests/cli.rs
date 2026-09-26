use super::*;
use crate::cli::Args;
use clap::error::ErrorKind;
use clap::Parser;

#[test]
fn no_flag_is_required() {
    let args = Args::try_parse_from(["bootc-anywhere"]).expect("the screen asks for every answer");
    assert_eq!(args.from, None);
    assert!(!args.no_tui);
}

#[test]
fn an_unknown_flag_is_refused() {
    let error = Args::try_parse_from(["bootc-anywhere", "--bogus"])
        .err()
        .expect("no such flag");
    assert_eq!(error.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn a_flag_missing_its_value_is_refused() {
    let error = Args::try_parse_from(["bootc-anywhere", "--disk"])
        .err()
        .expect("no value");
    assert_eq!(error.kind(), ErrorKind::InvalidValue);
}

#[test]
fn a_repeated_flag_is_refused() {
    let error = Args::try_parse_from(["bootc-anywhere", "--disk", "a", "--disk", "b"])
        .err()
        .expect("a repeated flag");
    assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
}

#[test]
fn every_flag_reaches_its_field() {
    let args = Args::try_parse_from([
        "bootc-anywhere",
        "--from",
        "/run/buildc-payload",
        "--disk=/dev/sda",
        "--hostname",
        "workstation",
        "--user",
        "me",
        "--password",
        "hunter2",
        "--encryption",
        "tpm2-luks",
        "--passphrase",
        "opensesame",
        "--pin",
        "123456",
        "--no-tui",
    ])
    .expect("every flag parses");
    assert_eq!(args.from.as_deref(), Some(Path::new("/run/buildc-payload")));
    assert_eq!(args.disk.as_deref(), Some("/dev/sda"));
    assert_eq!(args.hostname.as_deref(), Some("workstation"));
    assert_eq!(args.user.as_deref(), Some("me"));
    assert_eq!(args.password.as_deref(), Some("hunter2"));
    assert_eq!(args.encryption.as_deref(), Some("tpm2-luks"));
    assert_eq!(args.passphrase.as_deref(), Some("opensesame"));
    assert_eq!(args.pin.as_deref(), Some("123456"));
    assert!(args.no_tui);
}
