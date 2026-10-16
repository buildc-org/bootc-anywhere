use super::*;

pub(crate) const KINDS: [(&str, &str, &str); 5] = [
    (NONE, copy::ENC_NONE, ""),
    ("tpm2-luks", copy::ENC_TPM2, ""),
    ("luks-passphrase", copy::ENC_PASSPHRASE, ""),
    ("tpm2-luks-passphrase", copy::ENC_BOTH, ""),
    ("tpm2-luks-pin", copy::ENC_TPM2_PIN, ""),
];

pub(crate) const NONE: &str = "none";

pub(crate) const TPM: &str = "/dev/tpmrm0";

pub(crate) fn tpm() -> PathBuf {
    std::env::var_os("BUILDC_TPM").map_or_else(|| PathBuf::from(TPM), PathBuf::from)
}

const SYS_BLOCK: &str = "/sys/block";

pub(crate) fn sys_block() -> PathBuf {
    std::env::var_os("BUILDC_SYS_BLOCK").map_or_else(|| PathBuf::from(SYS_BLOCK), PathBuf::from)
}

const DEV: &str = "/dev";

pub(crate) fn dev() -> PathBuf {
    std::env::var_os("BUILDC_DEV").map_or_else(|| PathBuf::from(DEV), PathBuf::from)
}

const TTY0_ACTIVE: &str = "/sys/class/tty/tty0/active";

pub(crate) fn tty0_active() -> PathBuf {
    std::env::var_os("BUILDC_TTY0_ACTIVE").map_or_else(|| PathBuf::from(TTY0_ACTIVE), PathBuf::from)
}

const EFIVARS: &str = "/sys/firmware/efi/efivars";

pub(crate) fn efivars() -> PathBuf {
    std::env::var_os("BUILDC_EFIVARS").map_or_else(|| PathBuf::from(EFIVARS), PathBuf::from)
}
