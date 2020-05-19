//! An implementation of git rev-parse
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use thiserror::Error;

use crate::objects::{Id, Repo};
use crate::util::GitPath;

/// Errors that can be encountered while working with revs
#[derive(Debug, Error)]
pub enum RevError {
    /// The given reference cannot be resolved to a single commit
    #[error("Ambiguous reference {0}")]
    Ambiguous(String),

    /// We failed to find the ID pointed to by the rev
    #[error("Failed to find value of rev {0}")]
    Dangling(String),

    /// Invalid: the ref name fails the checking rules from
    /// `man git-check-ref-format`
    #[error("Invalid rev {0}")]
    Invalid(PathBuf),
}

/// check if a given string *could* be a SHA1
fn is_valid_sha1(s: &str) -> bool {
    // SHA1s must have 4-20 characters
    (4..=20).contains(&s.len())
        // SHA1s must be made of hex digits
        && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Parsing a rev file can either produce a symref pointer or an Id
enum RevParseResult {
    Symref(String),
    Id(Id),
}

/// Parse an Id from the given path. Caller should verify that symrefs *only*
/// occur in HEAD.
fn parse_id_from(path: &Path) -> Option<RevParseResult> {
    // we choose to steamroll through *all* OS errors and just return None
    let read = fs::read(path).ok()?;
    let read = String::from_utf8(read).ok()?;
    if let Some(target) = read.strip_prefix("ref:") {
        // symref
        Some(RevParseResult::Symref(target.trim().to_owned()))
    } else {
        Id::from(&read.trim_end()).map(|id| RevParseResult::Id(id))
    }
}

/// Checks if a refname is valid. See `man git-check-ref-format`.
fn is_valid_refname(name: &str, allow_onelevel: bool) -> bool {
    // cannot be @
    if name == "@" {
        return false;
    }

    let mut max = 0;
    for part in name.split('/') {
        // may not start or end with /
        if part == "" {
            return false;
        }

        // components may not start or end with . and must not end with .lock
        if part.starts_with('.') || part.ends_with('.') || part.ends_with(".lock") {
            return false;
        }

        // cannot contain .. or @{
        if part.contains("..") || part.contains("@{") {
            return false;
        }

        for ch in part.chars() {
            // cannot be ascii control characters or DEL or space, tilde, caret,
            // colon, question mark, star, open square bracket or backslash
            if ch < 0o040 as char || ch == 0o177 as char || " ~^:?*[\\".contains(ch) {
                return false;
            }
        }
        max += 1;
    }

    if max == 1 && !allow_onelevel {
        return false;
    }
    true
}

#[derive(Debug, Error)]
enum FollowSymlinkError {
    /// Too many symlinks were followed resolving this ref
    #[error("Symlink follow max depth exceeded resolving {0}")]
    DepthExceeded(PathBuf),

    /// A symlink target not of the format refs/... was encountered. Also,
    /// invalid UTF-8.
    #[error("Invalid symlink reference target {0}")]
    InvalidSymlinkRef(PathBuf),

    /// Encountered an unexpected IO error
    #[error("Unexpected IO failure {0}")]
    IOError(std::io::Error),
}

/// Follows symlink refs with targets of the format refs/... and return the ref
/// we finally hit. The resulting ref may not exist. Returns a PathBuf or a
/// RevError. Please give this function a .git-relative path.
fn follow_symlink_refs(
    p: &Path,
    dotgit: &Path,
) -> std::result::Result<PathBuf, FollowSymlinkError> {
    const MAX_DEPTH: usize = 16;

    assert!(
        !p.starts_with(dotgit),
        "follow_symlink_refs given an absolute path"
    );
    let mut path = p.to_owned();
    for depth in 0..MAX_DEPTH {
        let absolute = dotgit.join(&path);
        let stringified = path
            .to_git_path()
            .ok_or_else(|| FollowSymlinkError::InvalidSymlinkRef(path.clone()))?;

        trace!("stringified {:?}", &stringified);
        // only allow one level lookups on initial pass i.e. NEVER in a symlink
        // target
        if !is_valid_refname(&stringified, depth < 1) {
            return Err(FollowSymlinkError::InvalidSymlinkRef(path).into());
        }

        // if the path does not exist but is otherwise valid, we can still write
        // there
        if !absolute.exists() {
            return Ok(path);
        }

        // allow errors in getting the path metadata to halt evaluation
        let typ = absolute
            .symlink_metadata()
            .map_err(|e| FollowSymlinkError::IOError(e))?
            .file_type();

        if !typ.is_symlink() {
            return Ok(path);
        }

        path = absolute
            .read_link()
            .map_err(|e| FollowSymlinkError::IOError(e))?;
    }
    Err(FollowSymlinkError::DepthExceeded(p.to_owned()).into())
}

/// Updates the given reference to the new value. Follows symrefs in HEAD.
pub fn update_ref(target_ref: &Path, new_id: &Id, dotgit: &Path) -> Result<()> {
    // handle symrefs in HEAD
    let target_ref = if target_ref == Path::new("HEAD") {
        let head_path = dotgit.join("HEAD");
        let head_is_linkref = head_path.symlink_metadata()?.file_type().is_symlink();
        if head_is_linkref {
            follow_symlink_refs(&head_path, dotgit)?
        } else {
            match parse_id_from(&head_path) {
                // if there is an id in HEAD, we can continue to overwrite it
                Some(RevParseResult::Id(_)) => target_ref.to_owned(),
                // *however* if HEAD points to another reference, we should try
                // updating that one
                Some(RevParseResult::Symref(sr)) => PathBuf::from(sr),
                // if there is *nothing* in HEAD, let downstream handle this
                // error case
                None => target_ref.to_owned(),
            }
        }
    } else {
        target_ref.to_owned()
    };

    let stringified = target_ref
        .to_str()
        .ok_or_else(|| RevError::Invalid(target_ref.clone()))?;
    if !is_valid_refname(stringified, true) {
        return Err(RevError::Invalid(target_ref.clone()).into());
    }

    let try_paths = [
        ("", ""),
        ("refs/", ""),
        ("refs/tags/", ""),
        ("refs/heads/", ""),
        ("refs/remotes/", ""),
        ("refs/remotes/", "/HEAD"),
    ];
    for (before, after) in try_paths.iter() {
        let mut relative = PathBuf::from(before);
        relative.push(&target_ref);

        // prevent trailing slashes from getting added. But this is terrible and
        // there is probably a better way.
        if *after != "" {
            relative.push(after);
        }
        let absolute = dotgit.join(&relative);
        if !absolute.exists() {
            continue;
        }

        // ?? error handling here may be inconsistent with C git which probably
        // ignores erroneous link refs

        // follow link refs, may just get us p again. It is also acceptable if
        // the target does not exist here.
        let target = follow_symlink_refs(&relative, dotgit)?;
        // TODO: safe replacement of the file
        debug!("overwriting reference, writing to {}", absolute.display());
        fs::write(dotgit.join(target), format!("{}", new_id))?;
        return Ok(())
    }
    // if we fail to find somewhere to put the ref, assume it is new and
    // goes in .git.
    let absolute = dotgit.join(target_ref);
    debug!("new reference, writing to {}", absolute.display());
    fs::write(&absolute, format!("{}", new_id))?;
    Ok(())
}

/// Find the value of a refname in the .git directory
fn find_refname(rev: &str, dotgit: &Path) -> Option<Id> {
    // TODO: verify the rev name to ensure it doesn't have evil in it (see
    // `man git-check-ref-format`). Function implemented for this. Also should follow
    // symlinks properly.
    trace!("finding ref: {}", rev);
    let try_paths = ["", "refs", "refs/tags", "refs/heads", "refs/remotes"];
    for &path in try_paths.iter() {
        let mut p = dotgit.join(path);
        p.push(rev);
        trace!("=> trying {}", &p.display());

        return match parse_id_from(&p) {
            Some(RevParseResult::Id(id)) => Some(id),
            Some(RevParseResult::Symref(symref)) => {
                // Symrefs are invalid in any cases except if the rev is HEAD
                // This prevents infinite loops.
                if rev == "HEAD" {
                    trace!("=> found symref to {}", &symref);
                    find_refname(&symref, dotgit)
                } else {
                    None
                }
            }
            None => continue,
        };
    }

    // special case: refs/remotes/<refname>/HEAD
    let mut p = dotgit.join("refs/remotes");
    p.push(rev);
    p.push("HEAD");
    // This can't be a refname since it is not HEAD
    match parse_id_from(&p) {
        Some(RevParseResult::Id(id)) => Some(id),
        _ => None,
    }
}

/// Parse a revision identifier to attempt to find a unique id
pub fn parse(rev: &str, repo: &Repo) -> Result<Id> {
    if is_valid_sha1(rev) {
        // first, look for the SHA1 if it could be one
        let objectsdir = repo.root.join("objects");
        let firsttwodir = objectsdir.join(&rev[..=1]);
        if firsttwodir.is_dir() {
            // first two digits are present in the local objects directory

            let mut candidate = None;
            for file in firsttwodir.read_dir()? {
                // name decode errors mean that the file definitely doesn't
                // match the rev
                let file = file?;
                if file
                    .file_name()
                    .to_str()
                    .map(|name| name.starts_with(rev))
                    .unwrap_or(false)
                {
                    // already found a candidate, so this must be ambiguous
                    if candidate.is_some() {
                        return Err(RevError::Ambiguous(rev.to_owned()).into());
                    }
                    // recreate the full hash
                    candidate = Some(rev[..=1].to_owned() + file.file_name().to_str().unwrap());
                }
            }
            if let Some(ret) = candidate {
                // this is not a reasonably catchable error by consumers and is
                // a case of invalid stuff happening with the repo
                return Id::from(&ret).with_context(|| format!("invalid id filename {}", ret));
            }
        }
    }

    // TODO: § <describeOutput> https://git-scm.com/docs/git-rev-parse

    // <refname>
    if let Some(id) = find_refname(rev, &repo.root) {
        return Ok(id);
    }

    // @ represents HEAD
    if rev == "@" {
        return repo.head();
    }

    Err(RevError::Dangling(rev.to_owned()).into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_validate_refname() {
        let expected_responses = [
            ("abc", false),
            ("a/c", true),
            ("a//b", false),
            ("a/", false),
            ("a/../b", false),
            ("a/.a/b", false),
            ("a/a.lock/b", false),
            ("a\x01/b", false),
            ("a/a?a/b", false),
            ("ab@{c", false),
        ];

        for (inp, expect) in expected_responses.iter() {
            assert_eq!(super::is_valid_refname(inp, false), *expect);
        }
        assert_eq!(super::is_valid_refname("abc", true), true);
    }
}
