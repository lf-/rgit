//! Helpers for simplifying commonly-used patterns in Git
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
    /// spite of being invalid UTF-8. We reject all paths that contain these,
    /// which is a calculated incompatibility with C git.
    ///
    /// This function copies the path into a new String.
    fn to_git_path(&self) -> Option<String>;
}

impl GitPath for Path {
    fn to_git_path(&self) -> Option<String> {
        // OsStr is too opaque to directly replace the slashes :(
        let mut parts = Vec::new();
        for part in self.iter() {
            parts.push(part.to_str()?);
        }
        Some(parts.join("/"))
    }
}

/// Prints a bytes string with all non-ascii characters in escaped format
#[allow(unused)]
pub(crate) fn to_bytes_literal(s: &[u8]) -> String {
    let mut res = String::new();
    for &c in s {
        res.extend(ascii::escape_default(c).map(|b| b as char));
    }
    res
}

#[cfg(test)]
mod test {
    use super::GitPath;
    use std::path::Path;

    #[test]
    fn test_git_path() {
        let path = Path::new("a/b");
        assert_eq!(path.to_git_path().unwrap(), "a/b");
        let path = Path::new("a");
        assert_eq!(path.to_git_path().unwrap(), "a");
    }
}
