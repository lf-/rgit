use std::ascii;
use std::fmt;

/// A byte string wrapped in a type
pub struct ByteString(pub Vec<u8>);

impl fmt::Display for ByteString {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", to_bytes_literal(&self.0))
    }
}

impl fmt::Debug for ByteString {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "b\"{}\"", self)
    }
}

impl std::ops::Deref for ByteString {
    type Target = Vec<u8>;
    fn deref(&self) -> &Self::Target {
        &self.0
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
