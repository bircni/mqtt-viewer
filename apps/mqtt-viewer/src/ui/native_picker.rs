//! The operating system's file dialogs.

use std::path::PathBuf;

use crate::picker::FilePicker;

/// Opens the platform's native open and save dialogs. They block until the user answers.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativePicker;

impl FilePicker for NativePicker {
    fn open(&self, kind: &str, extensions: &[&str]) -> Option<PathBuf> {
        rfd::FileDialog::new().add_filter(kind, extensions).pick_file()
    }

    fn save(&self, default_name: &str, kind: &str, extensions: &[&str]) -> Option<PathBuf> {
        rfd::FileDialog::new().set_file_name(default_name).add_filter(kind, extensions).save_file()
    }
}
