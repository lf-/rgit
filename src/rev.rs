//! An implementation of git rev-parse
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use thiserror::Error;

use crate::objects::{Id, Repo};

/// Errors that can be encountered while working with revs
#[derive(Debug, Error)]
pub enum RevError {
    /// The given reference cannot be resolved to a single commit
    #[error("Ambiguous reference {0}")]
    Ambiguous(String),

    /// We failed to find the ID pointed to by the rev
    #[error("Failed to find value of rev {0}")]
    NotFound(String),
}

/// check if a given string *could* be a SHA1
fn is_valid_sha1(s: &str) -> bool {
    // SHA1s must have 4-20 characters
    (4..=20).contains(&s.len())
        // SHA1s must be made of hex digits
        && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Parse an Id from the given path
fn parse_id_from(path: &Path) -> Option<Id> {
    let read = fs::read(path);
    // we choose to steamroll through *all* OS errors and just return None
    read.ok()
        .and_then(|v| Id::from(std::str::from_utf8(&v).ok()?.trim_end()))
}

/// Find a refname in the .git directory
fn find_refname(rev: &str, dotgit: &Path) -> Option<Id> {
    let try_paths = ["", "refs", "refs/tags", "refs/heads", "refs/remotes"];
    for &path in try_paths.iter() {
        let mut p = dotgit.join(path);
        p.push(rev);

        if let Some(id) = parse_id_from(&p) {
            return Some(id);
        }
    }

    // special case: refs/remotes/<refname>/HEAD
    let mut p = dotgit.join("refs/remotes");
    p.push(rev);
    p.push("HEAD");
    parse_id_from(&p)
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
        return repo
            .head()
            .and_then(|inner| inner.context("repo has no HEAD"));
    }

    Err(RevError::NotFound(rev.to_owned()).into())
}
