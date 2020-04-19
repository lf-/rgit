use crate::objects::Id;
use crate::util::ByteString;
use anyhow::{Error, Result};
use safecast::Safecast;
use std::fmt;
use std::fs;
use std::io;
use std::mem;
use std::path::Path;
use std::time;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum IndexError {
    #[error("Unsupported index version {0}")]
    UnsupportedVersion(u32),

    #[error("Bad header magic")]
    BadMagic,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
#[allow(non_camel_case_types)]
struct u32be([u8; 4]);

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

// Safety: u32be is composed of a slice of a Safecast type that requires no
// further checking. It must be #[repr(transparent)] because of this impl.
unsafe impl Safecast for u32be {
    fn safecast(&self) {}
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
#[allow(non_camel_case_types)]
struct u16be([u8; 2]);

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

// Safety: u16be is composed of a slice of a Safecast type that requires no
// further checking. It must be #[repr(transparent)] because of this impl.
unsafe impl Safecast for u16be {
    fn safecast(&self) {}
}

impl fmt::Debug for u16be {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let numeric: u16 = self.clone().into();
        fmt::Debug::fmt(&numeric, f)
    }
}

// Safety: Id is a slice of bytes which is Safecast-able
unsafe impl Safecast for Id {
    fn safecast(&self) {}
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

#[derive(Safecast, Clone, Debug)]
#[repr(C)]
/// order: sorted in ascending order on name field, sorted in byte comparison order
pub struct IndexEntry {
    ctime: u32be,
    ctime_ns: u32be,

    mtime: u32be,
    mtime_ns: u32be,

    dev: u32be,
    ino: u32be,

    /// [31:16] unused, left zero; unaccounted for in docs (?????)
    /// [15:12] object type: regular file (0b1000); symbolic link (0b1010);
    ///         gitlink (0b1110)
    /// [11:9] unused, left zero
    /// [8:0] unix permission: 0o0755 or 0o0644 for files, symlinks are 0
    mode: u32be,

    uid: u32be,
    gid: u32be,

    size: u32be,

    id: Id,

    /// [16] assume-valid flag
    /// [15] extended flag
    /// [14:13] stage
    /// [12:0] name length or 0xFFF (if name is longer)
    flags: u16be,
    // TODO: added in v3 but we choose not to implement that yet
    //
    // /// [16] reserved
    // /// [15] skip-worktree flag
    // /// [14] intent-to-add (related to git add --patch)
    // /// [13:0] unused; must be zero
    // eflags: u16be,
}

/// Files indexed in this index. Must be kept sorted.
type Index = Vec<(ByteString, IndexEntry)>;

pub(crate) fn parse(mut file: impl io::Read) -> Result<Index> {
    let mut buf = [0u8; mem::size_of::<Header>()];
    file.read_exact(&mut buf)?;

    let header = buf.cast::<Header>();
    let header = &header[0];

    trace!("index header {:?}", &header);

    if &header.signature != b"DIRC" {
        return Err(Error::new(IndexError::BadMagic));
    }

    let ver = header.version.into();
    if ver != 2 {
        return Err(Error::new(IndexError::UnsupportedVersion(ver)));
    }

    let num_entries = u32::from(header.num_entries) as usize;
    let mut buf = [0u8; mem::size_of::<IndexEntry>()];
    let mut name = Vec::new();

    let mut files = Vec::with_capacity(num_entries);

    for _ in 0..num_entries {
        file.read_exact(&mut buf)?;
        let entry = buf.cast::<IndexEntry>();
        let entry: &IndexEntry = &entry[0];

        // bottom 12 bits of flags is name size
        let flags: u16 = entry.flags.into();
        let sz = (flags & 0xfff) as usize;
        if sz == 0xfff {
            // must be measured manually. implementation not today
            unimplemented!("name is >0xfff characters long. unsupported");
        }

        // pad record incl name + nul byte to 8 byte boundary
        let full_record_sz = mem::size_of::<IndexEntry>() + sz;
        let full_record_sz = full_record_sz + (8 - full_record_sz % 8);
        let record_sz = full_record_sz - mem::size_of::<IndexEntry>();

        // we deliberately choose to keep the vector at the size of the longest name
        if name.capacity() < record_sz {
            name.resize_with(record_sz, Default::default);
        }

        file.read_exact(&mut name[..record_sz])?;
        files.push((ByteString(Vec::from(&name[..sz])), entry.clone()))
    }

    Ok(files)
}

/// Converts a SystemTime object to a (secs, nsecs) tuple of time since the Unix
/// epoch
fn system_time_to_epoch(systime: time::SystemTime) -> Result<(u64, u32)> {
    let dur = systime.duration_since(time::UNIX_EPOCH)?;
    Ok((dur.as_secs(), dur.subsec_nanos()))
}

/// Checks the OS-specific stuff in the index entry to be equal
/// On Windows, we ignore:
/// - file mode (consider to be equal)
/// - dev, ino
/// - uid, gid
#[cfg(target_family = "windows")]
fn is_same_os(_meta: &fs::Metadata, _entry: &IndexEntry) -> Result<bool> {
    Ok(true)
}

#[cfg(target_family = "unix")]
fn is_same_os(meta: &fs::Metadata, entry: &IndexEntry) -> Result<bool> {
    todo!("Sorry, we've not implemented extended metadata checking for Linux yet")
}

/// Checks metadata to find if a file is highly likely to be the same as the one
/// in the index without having to read and hash it
pub(crate) fn get_id(file: &Path, entry: &IndexEntry) -> Result<bool> {
    // XXX: these u32 timestamps will panic after 2038 but git will break too 🤷‍♀️
    let meta = fs::metadata(file)?;

    let modtime = system_time_to_epoch(meta.modified()?)?;
    if u32::from(entry.mtime) != modtime.0 as u32 || u32::from(entry.mtime_ns) != modtime.1 {
        return Ok(false);
    }

    let createtime = system_time_to_epoch(meta.created()?)?;
    if u32::from(entry.ctime) != createtime.0 as u32 || u32::from(entry.ctime_ns) != createtime.1 {
        return Ok(false);
    }

    let size = meta.len();
    if u32::from(entry.size) as u64 != size {
        return Ok(false);
    }

    is_same_os(&meta, entry)
}
