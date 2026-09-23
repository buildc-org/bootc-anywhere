use std::path::Path;

const SEARCHED: [&str; 3] = [
    "/usr/lib64",
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
];

const SONAME: &str = "libfdisk.so.1";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for directory in SEARCHED {
        let library = Path::new(directory).join(SONAME);
        if library.exists() {
            println!("cargo:rustc-link-arg={}", library.display());
            return;
        }
    }
    panic!("{SONAME} is in none of {SEARCHED:?}, so the partition backend cannot link");
}
