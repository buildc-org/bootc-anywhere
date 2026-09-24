pub mod copy;
pub mod firmware;

pub(crate) use common::json::{self, Json};
pub(crate) use common::prompt::Prompt;
pub(crate) use common::ui::Choice;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::process::{Command, Stdio};

mod answers;
mod backend;
mod boot;
mod collect;
mod discover;
mod disks;
mod editor;
mod env;
mod esp;
mod etcwrite;
mod fdisk;
mod form;
mod lock;
mod panel;
mod payload;
mod recipe;
mod run;
mod table;
mod volumes;

pub use answers::*;
pub(crate) use backend::*;
pub(crate) use boot::*;
pub(crate) use discover::*;
pub use disks::*;
pub(crate) use editor::*;
pub(crate) use env::*;
pub(crate) use esp::*;
pub(crate) use etcwrite::*;
pub(crate) use form::*;
pub use lock::*;
pub(crate) use panel::*;
pub use payload::*;
pub use recipe::*;
pub use run::*;
pub(crate) use table::*;
pub(crate) use volumes::*;

pub const PROGRAM: &str = "bootc-anywhere";

pub const REPO_FILE: &str = "repo.kdl";

pub const BUILD_MANIFEST: &str = "/usr/share/buildc/manifest.json";

#[cfg(test)]
mod tests;
