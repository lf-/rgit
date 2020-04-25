use std::ascii;
use std::path::Path;

/// A path in Git format: UTF-8 with forward slash as delimiter
pub trait GitPath {
    /// Stringifies a path in Git format
    ///
    /// Git stores paths in UTF-8 normalization form "C". It does not accept
    /// unpaired surrogate byte sequences, so we can use normal encoding
    /// functions to handle them.
    ///
    /// Note that there is one known inconsistency with this: on Unix platforms,
    /// lone continuation bytes in filenames are tolerated by Git (?????) in
    /// spite of being invalid UTF-8. We choose deliberately to ignore this
    /// and just put U+FFFD REPLACEMENT CHARACTERs instead of any illegal
    /// characters.
    ///
    /// This function copies the path into a new String.
    fn to_git_path(&self) -> String;
}

impl GitPath for Path {
    fn to_git_path(&self) -> String {
        let parts: Vec<_> = self.iter().map(|comp| comp.to_string_lossy()).collect();
        parts.join("/")
    }
}

/// Prints a bytes string that may contain invalid UTF-8 in escaped format
pub(crate) fn to_bytes_literal(s: &[u8]) -> String {
    let mut res = String::new();
    for &c in s {
        res.extend(ascii::escape_default(c).map(|b| b as char));
    }
    res
}
