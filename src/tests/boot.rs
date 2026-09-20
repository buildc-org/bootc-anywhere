use super::*;

#[test]
fn a_boot_argument_lands_on_the_options_line_once() {
    let raw = "title Fedora\nlinux /vmlinuz\noptions root=UUID=aa quiet\n";
    let (text, changed) = add_boot_arg(raw, "rd.luks.name=bb=root");
    assert!(changed);
    assert!(text.contains("options root=UUID=aa quiet rd.luks.name=bb=root"));
    assert!(text.ends_with('\n'));
    let (again, changed) = add_boot_arg(&text, "rd.luks.name=bb=root");
    assert!(!changed);
    assert_eq!(again, text);
    assert!(boot_arg_named(&text, "rd.luks.name=bb=root"));
    assert!(!boot_arg_named(raw, "rd.luks.name=bb=root"));
    assert!(!boot_arg_named("title nothing\n", "rd.luks.name=bb=root"));
}
