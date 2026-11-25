//! Asking the user for a file. A trait so that everything behind the native dialog can be tested without one.

use std::fmt::Debug;
use std::path::PathBuf;

/// Lets the user choose files.
pub trait FilePicker: Debug {
    /// Asks for an existing file with one of `extensions`; `None` when the user cancels.
    fn open(&self, kind: &str, extensions: &[&str]) -> Option<PathBuf>;

    /// Asks where to save a file, suggesting `default_name`; `None` when the user cancels.
    fn save(&self, default_name: &str, kind: &str, extensions: &[&str]) -> Option<PathBuf>;
}
