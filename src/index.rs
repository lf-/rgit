//! Low-level functions for working with an index
use crate::objects::{Blob, Id, Object, Repo};
use anyhow::{Context, Error, Result};
use safecast::Safecast;
use sha1::{Digest, Sha1};
use std::fmt;
use std::fs;
use std::io;
use std::mem;
use std::path::Path;
use std::time;
use thiserror::Error;

const SIGNATURE: [u8; 4] = *b"DIRC";
const VERSION: u32 = 2;

/// Files indexed in this index. Must be kept sorted.
pub type Index = Vec<IndexEntry>;

#[derive(Error, Debug)]
pub enum IndexError {
    #[error("Unsupported index version {0}")]
    UnsupportedVersion(u32),

    #[error("Bad header magic")]
    BadMagic,
}

#[derive(Safecast, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
#[allow(non_camel_case_types)]
pub struct u32be([u8; 4]);

impl From<u32be> for u32 {
    fn from(be: u32be) -> Self {
        u32::from_be_bytes(be.0)
    }
}

impl From<u32> for u32be {
    fn from(num: u32) -> Self {
        u32be(num.to_be_bytes())
    }
}

impl fmt::Debug for u32be {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let numeric: u32 = self.clone().into();
        fmt::Debug::fmt(&numeric, f)
    }
}

#[derive(Safecast, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
#[allow(non_camel_case_types)]
pub struct u16be([u8; 2]);

impl From<u16be> for u16 {
    fn from(be: u16be) -> Self {
        u16::from_be_bytes(be.0)
    }
}

impl From<u16> for u16be {
    fn from(num: u16) -> Self {
        u16be(num.to_be_bytes())
    }
}

impl fmt::Debug for u16be {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let numeric: u16 = self.clone().into();
        fmt::Debug::fmt(&numeric, f)
    }
}

#[derive(Safecast, Debug)]
#[repr(C)]
struct Header {
    /// b"DIRC" literal
    signature: [u8; 4],

    /// 4-byte version number:
    ///   The current supported versions are 2, 3 and 4.
    version: u32be,

    num_entries: u32be,
}

/// Entry in the Git index
/// order: sorted in ascending order on name field, sorted in byte comparison order
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub name: String,
    pub meta: IndexMeta,
}

#[derive(Safecast, Clone, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct IndexMeta {
    pub ctime: u32be,
    pub ctime_ns: u32be,

    pub mtime: u32be,
    pub mtime_ns: u32be,

    pub dev: u32be,
    pub ino: u32be,

    /// \[31:16\] unused, left zero; unaccounted for in docs (?????)
    /// \[15:12\] object type: regular file (0b1000); symbolic link (0b1010);
    ///         gitlink (0b1110)
    /// \[11:9\] unused, left zero
    /// \[8:0\] unix permission: 0o0755 or 0o0644 for files, symlinks are 0
    pub mode: u32be,

    pub uid: u32be,
    pub gid: u32be,

    pub size: u32be,

    pub id: Id,

    /// \[16\] assume-valid flag
    /// \[15\] extended flag
    /// \[14:13\] stage
    /// \[12:0\] name length or 0xFFF (if name is longer)
    pub flags: u16be,
    // TODO: added in v3 but we choose not to implement that yet
    //
    // /// \[16\] reserved
    // /// \[15\] skip-worktree flag
    // /// \[14\] intent-to-add (related to git add --patch)
    // /// \[13:0\] unused; must be zero
    // eflags: u16be,
}

#[derive(Debug, PartialEq)]
pub struct StatInfo {
    pub ctime: (u32, u32),
    pub mtime: (u32, u32),
    pub size: u32,
    pub unix_stat: UnixStat,
}

#[derive(Debug, Default)]
pub struct UnixStat {
    pub dev: u32,
    pub ino: u32,
    pub uid: u32,
    pub gid: u32,
    pub executable: bool,
}

impl UnixStat {
    /// Gets the unix-specific stat stuff. Not implemented on Unix yet but zero
    /// is an acceptable value
    fn get(_meta: &fs::Metadata) -> UnixStat {
        Default::default()
    }

    fn mode(&self) -> u32 {
        if self.executable {
            0o100755
        } else {
            0o100644
        }
    }
}

impl StatInfo {
    fn get(path: &Path) -> Result<StatInfo> {
        // XXX: these u32 timestamps will break after 2038 but git will break too 🤷‍♀️
        let meta = fs::metadata(path).with_context(|| {
            format!(
                "failed to find metadata for {} while making index",
                path.display()
            )
        })?;

        let mtime = system_time_to_epoch(meta.modified()?)?;
        let ctime = system_time_to_epoch(meta.modified()?)?;

        let size = meta.len() as u32;

        let unix_stat = UnixStat::get(&meta);
        Ok(StatInfo {
            mtime,
            ctime,
            size,
            unix_stat,
        })
    }
}

impl PartialEq for UnixStat {
    /// Compares a UnixStat, ignoring fields if they are zero
    fn eq(&self, other: &UnixStat) -> bool {
        (self.dev == 0 || self.dev == other.dev)
            && (self.gid == 0 || other.gid == 0 || self.gid == other.gid)
            && (self.uid == 0 || other.uid == 0 || self.uid == other.uid)
            && (self.ino == 0 || other.ino == 0 || self.ino == other.ino)
            && (self.dev == 0 || other.dev == 0 || self.dev == other.dev)
            && (cfg!(not(target_family = "unix")) || self.executable == other.executable)
    }
}

impl IndexEntry {
    /// Checks if a file in the index has changed since it was added to the index
    pub fn is_same_as_tree(&self, repo: &Repo) -> Result<bool> {
        let filepath = &repo.tree_root().join(&self.name);
        let si = StatInfo::get(&filepath)
            .with_context(|| format!("finding filesystem stats for {}", filepath.display()))?;

        if self.meta.statinfo() == si {
            // the stat info is the same. Unless people are playing tricks on us
            // (that's their fault) these files will be identical given
            // identical mtime, ctime
            return Ok(true);
        }

        // if they are in fact different, we need to expensively check whether
        // the hashes of the files are the same
        let blob = Blob::new_from_disk(&filepath)
            .with_context(|| format!("making a blob of {}", filepath.display()))?;
        let (id, _) = Object::prepare_store(&blob);
        Ok(id == self.meta.id)
    }
}

impl IndexMeta {
    /// Generates the metadata for a given file as it would be if it were added
    /// to the index
    pub fn new_from_file(filename: &str, repo: &Repo) -> Result<IndexMeta> {
        let path = repo.tree_root().join(filename);

        let id = repo.store(&Blob::new_from_disk(&path)?)?;
        let statinfo = StatInfo::get(&path)?;

        // bottom 12 bits of the name length are flags
        let flags = (filename.len() & 0xfff) as u16;

        trace!("making index entry for {}", filename);

        Ok(IndexMeta {
            ctime: statinfo.ctime.0.into(),
            ctime_ns: statinfo.ctime.1.into(),
            mtime: statinfo.mtime.0.into(),
            mtime_ns: statinfo.mtime.1.into(),

            size: statinfo.size.into(),
            id,
            flags: flags.into(),

            mode: statinfo.unix_stat.mode().into(),
            dev: statinfo.unix_stat.dev.into(),
            ino: statinfo.unix_stat.ino.into(),
            uid: statinfo.unix_stat.uid.into(),
            gid: statinfo.unix_stat.gid.into(),
        })
    }

    pub fn statinfo(&self) -> StatInfo {
        StatInfo {
            ctime: (self.ctime.into(), self.ctime_ns.into()),
            mtime: (self.mtime.into(), self.mtime_ns.into()),
            size: self.size.into(),
            unix_stat: UnixStat {
                dev: self.dev.into(),
                ino: self.ino.into(),
                uid: self.uid.into(),
                gid: self.gid.into(),
                executable: (u32::from(self.mode) & ((1 << 9) - 1)) == 0o755,
            },
        }
    }
}

/// Ensure a file is in an index. `filename` is a repo-relative path.
pub fn add_to_index(index: &mut Index, filename: &str, repo: &Repo) -> Result<Id> {
    let existing_entry =
        index.binary_search_by(|IndexEntry { name, .. }| name.as_str().cmp(filename));

    let path = repo.tree_root().join(filename);
    let filestats = StatInfo::get(&path)?;

    Ok(match existing_entry {
        // If it's in the index and all the stats are the same, we can assume
        // it's the same and no-op
        Ok(found) if index[found].meta.statinfo() == filestats => index[found].meta.id.clone(),

        // It's in the index but the entry is old. Replace the entry. This will
        // no-op if the file has been modified but has the same contents
        Ok(found) => {
            let new_entry = IndexMeta::new_from_file(filename, repo)?;
            let id = new_entry.id.clone();
            index[found].meta = new_entry;
            id
        }

        // Not in the index
        Err(idx) => {
            let new_entry = IndexMeta::new_from_file(filename, repo)?;
            let id = new_entry.id.clone();
            index.insert(
                idx,
                IndexEntry {
                    name: filename.to_string(),
                    meta: new_entry,
                },
            );
            id
        }
    })
}

pub fn write_to_file(index: &Index, mut file: impl io::Write) -> Result<()> {
    let mut hash = Sha1::new();
    let header = Header {
        signature: SIGNATURE,
        version: VERSION.into(),
        num_entries: (index.len() as u32).into(),
    };
    let header_buf = header.cast();
    file.write_all(header_buf)?;
    hash.input(header_buf);

    for IndexEntry { name, meta } in index {
        let entry_buf = meta.cast();
        file.write_all(entry_buf)?;
        hash.input(entry_buf);

        // Figure out how long the name field is then produce padding to write
        // after the name to make it that length
        let namerecsz = name_record_size(name.len());
        let padding_zeros = vec![0u8; namerecsz - name.len()];

        file.write_all(name.as_bytes())?;
        file.write_all(&padding_zeros)?;
        hash.input(&name);
        hash.input(&padding_zeros);
    }

    // write a hash of the contents at the end of the file
    let res: [u8; 20] = hash.result().into();
    file.write_all(&res)?;
    Ok(())
}

/// Finds the number of bytes that the name record in the index will occupy (with padding)
fn name_record_size(name_length: usize) -> usize {
    // pad record incl name + nul byte to 8 byte boundary
    let full_record_sz = mem::size_of::<IndexMeta>() + name_length;
    let full_record_sz = full_record_sz + (8 - full_record_sz % 8);
    full_record_sz - mem::size_of::<IndexMeta>()
}

/// Reads an index out of a file
pub(crate) fn parse(mut file: impl io::Read) -> Result<Index> {
    let mut buf = [0u8; mem::size_of::<Header>()];
    file.read_exact(&mut buf)?;

    let header = buf.cast::<Header>();
    let header = &header[0];

    trace!("index header {:?}", &header);

    if header.signature != SIGNATURE {
        return Err(Error::new(IndexError::BadMagic));
    }

    let ver = header.version.into();
    if ver != 2 {
        return Err(Error::new(IndexError::UnsupportedVersion(ver)));
    }

    let num_entries = u32::from(header.num_entries) as usize;
    let mut name = Vec::new();

    let mut buf = [0u8; mem::size_of::<IndexMeta>()];
    let mut files = Vec::with_capacity(num_entries);

    for _ in 0..num_entries {
        file.read_exact(&mut buf)?;
        let entry = buf.cast::<IndexMeta>();
        let meta: &IndexMeta = &entry[0];

        // bottom 12 bits of flags is name size
        let flags: u16 = meta.flags.into();
        let name_length = (flags & 0xfff) as usize;
        if name_length == 0xfff {
            // must be measured manually. implementation not today
            unimplemented!("name is >0xfff characters long. unsupported");
        }

        let record_sz = name_record_size(name_length);

        // we deliberately choose to keep the vector at the size of the longest name
        if name.len() < record_sz {
            name.resize_with(record_sz, Default::default);
        }

        file.read_exact(&mut name[..record_sz])?;
        files.push(IndexEntry {
            name: std::str::from_utf8(&name[..name_length])?.to_string(),
            meta: meta.clone(),
        });
    }

    Ok(files)
}

/// Converts a SystemTime object to a (secs, nsecs) tuple of time since the Unix
/// epoch
fn system_time_to_epoch(systime: time::SystemTime) -> Result<(u32, u32)> {
    let dur = systime.duration_since(time::UNIX_EPOCH)?;
    Ok((dur.as_secs() as u32, dur.subsec_nanos()))
}

#[cfg(test)]
mod tests {
    use super::{IndexEntry, IndexMeta};
    const TEST_INDEX: &[u8] = include_bytes!("testdata/test_index");
    const TEST_INDEX_TREE: &[u8] = include_bytes!("testdata/test_index_tree");

    #[test]
    fn test_index() {
        let index = vec![
            IndexEntry {
                name: "item1".to_string(),
                meta: IndexMeta {
                    ctime: 0x5e9bf1c6.into(),
                    ctime_ns: 0x26545c10.into(),
                    mtime: 0x5e9bf1ce.into(),
                    mtime_ns: 0x30640b74.into(),
                    dev: 0x0.into(),
                    ino: 0x0.into(),
                    mode: 0x81a4.into(),
                    uid: 0x0.into(),
                    gid: 0x0.into(),
                    size: 0x8.into(),
                    id: super::Id::from("07d4aba2654d6d44c24862467d86ee8eb67840fe").unwrap(),
                    flags: 0x5.into(),
                },
            },
            IndexEntry {
                name: "item2".to_string(),
                meta: IndexMeta {
                    ctime: 0x5e9bf1c9.into(),
                    ctime_ns: 0xb204508.into(),
                    mtime: 0x5e9bf1d2.into(),
                    mtime_ns: 0x2ce99284.into(),
                    dev: 0x0.into(),
                    ino: 0x0.into(),
                    mode: 0x81a4.into(),
                    uid: 0x0.into(),
                    gid: 0x0.into(),
                    size: 0xc.into(),
                    id: super::Id::from("0bfeb48f6e414e435fe4fbf1d85d5a3a83dd4251").unwrap(),
                    flags: 0x5.into(),
                },
            },
        ];

        let mut idx_buf = Vec::new();

        super::write_to_file(&index, &mut idx_buf).unwrap();

        assert_eq!(idx_buf, TEST_INDEX);

        let parsed = super::parse(TEST_INDEX).unwrap();
        assert_eq!(index, parsed);
    }

    #[test]
    #[ignore = "not yet implemented; need to add TREE extension first"]
    fn test_index_tree() {
        let index = vec![
            IndexEntry {
                name: "dir/item".to_string(),
                meta: IndexMeta {
                    ctime: 0x5e9bc19e.into(),
                    ctime_ns: 0x217b3358.into(),
                    mtime: 0x5e9bc19e.into(),
                    mtime_ns: 0x218a78b8.into(),
                    dev: 0x0.into(),
                    ino: 0x0.into(),
                    mode: 0x81a4.into(),
                    uid: 0x0.into(),
                    gid: 0x0.into(),
                    size: 0x14.into(),
                    id: super::Id::from("c2801012ebf8905049b7555a8e1a32fb2df68a8f").unwrap(),
                    flags: 0x8.into(),
                },
            },
            IndexEntry {
                name: "file2".to_string(),
                meta: IndexMeta {
                    ctime: 0x5e9bbee2.into(),
                    ctime_ns: 0x1b0f3be0.into(),
                    mtime: 0x5e9bbee2.into(),
                    mtime_ns: 0x1b1e8398.into(),
                    dev: 0x0.into(),
                    ino: 0x0.into(),
                    mode: 0x81a4.into(),
                    uid: 0x0.into(),
                    gid: 0x0.into(),
                    size: 0xc.into(),
                    id: super::Id::from("107f41d5f9e9ea48ff6a312917c9bb029cf9d2b6").unwrap(),
                    flags: 0x5.into(),
                },
            },
        ];

        let mut idx_buf = Vec::new();

        super::write_to_file(&index, &mut idx_buf).unwrap();
        assert_eq!(idx_buf, TEST_INDEX_TREE);
    }
}
