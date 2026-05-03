//! Deterministic single-file archive transport for blob stores.

use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::str;

use gbf_foundation::Hash256;
use sha2::{Digest, Sha256};

use crate::blob::{BlobStore, BlobStoreError};
use crate::gc::{BlobReferenceError, BlobReferencesRegistry};
use crate::pinset::{Pinset, PinsetName, PinsetNameError};

pub const ARCHIVE_MAGIC: [u8; 8] = *b"GBLM\0ARC";
pub const ARCHIVE_VERSION: u8 = 1;
pub const ARCHIVE_HEADER_LEN: usize = 24;
/// Maximum single-record body accepted by the in-memory archive reader.
///
/// The store can address larger blobs, but this transport currently reads one
/// blob body at a time before committing it. A streaming extractor can raise or
/// remove this cap later without changing the archive wire format.
pub const MAX_ARCHIVE_BLOB_BODY_LEN: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveHeader {
    pub magic: [u8; 8],
    pub version: u8,
    pub pinset_count: u16,
    pub blob_count: u32,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractedArchive {
    pub header: ArchiveHeader,
    pub pinsets: Vec<Pinset>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveContents {
    pub header: ArchiveHeader,
    pub pinsets: Vec<Pinset>,
    pub blobs: Vec<(Hash256, u64)>,
}

pub fn create_archive(
    store: &BlobStore,
    pinsets: &[Pinset],
    refs: &BlobReferencesRegistry,
    out: &mut dyn Write,
) -> Result<ArchiveHeader, ArchiveError> {
    let mut sorted_pinsets = pinsets.to_vec();
    sorted_pinsets.sort_by(|left, right| left.name.cmp(&right.name));

    validate_pinsets(&sorted_pinsets)?;
    let blobs = collect_archive_blobs(store, &sorted_pinsets, refs)?;
    let blob_count = u32::try_from(blobs.len()).map_err(|_| ArchiveError::TooManyBlobs {
        count: blobs.len(),
        max: u32::MAX,
    })?;

    let mut total_bytes = 0_u64;
    let mut blob_lengths = Vec::with_capacity(blobs.len());
    for hash in &blobs {
        let len = archive_blob_len(store, *hash)?;
        checked_body_len(len)?;
        copy_verified_blob(store, *hash, &mut io::sink())?;
        total_bytes = total_bytes
            .checked_add(len)
            .ok_or(ArchiveError::TotalBytesOverflow)?;
        blob_lengths.push((*hash, len));
    }

    let header = ArchiveHeader {
        magic: ARCHIVE_MAGIC,
        version: ARCHIVE_VERSION,
        pinset_count: sorted_pinsets.len() as u16,
        blob_count,
        total_bytes,
    };

    write_header(out, &header)?;
    for pinset in &sorted_pinsets {
        write_pinset(out, pinset)?;
    }
    for (hash, len) in blob_lengths {
        out.write_all(hash.as_bytes())?;
        write_u64(out, len)?;
        copy_verified_blob(store, hash, out)?;
    }

    Ok(header)
}

/// Extract an archive into `store`.
///
/// Extraction is not transactional: a malformed later record may leave earlier
/// extracted blobs in the store. Each blob record is hash-checked before it is
/// committed.
pub fn extract_archive(
    input: &mut dyn Read,
    store: &BlobStore,
) -> Result<ExtractedArchive, ArchiveError> {
    let header = read_header(input)?;
    let pinsets = read_pinsets(input, header.pinset_count)?;
    let mut remaining = header.total_bytes;

    for _ in 0..header.blob_count {
        let hash = read_hash(input)?;
        let len = read_u64(input)?;
        checked_record_len(len, remaining)?;
        let body = read_body(input, len)?;
        remaining -= len;
        put_archive_body(store, hash, &body)?;
    }

    if remaining != 0 {
        return Err(ArchiveError::TotalBytesMismatch {
            header: header.total_bytes,
            actual: header.total_bytes - remaining,
        });
    }
    reject_trailing_bytes(input)?;

    Ok(ExtractedArchive { header, pinsets })
}

/// List archive contents without writing into a store.
///
/// With a plain `Read`, this consumes every blob body to advance to the next
/// record and rejects trailing bytes after the declared records.
pub fn list_archive(input: &mut dyn Read) -> Result<ArchiveContents, ArchiveError> {
    let header = read_header(input)?;
    let pinsets = read_pinsets(input, header.pinset_count)?;
    let mut blobs = Vec::new();
    let mut remaining = header.total_bytes;

    for _ in 0..header.blob_count {
        let hash = read_hash(input)?;
        let len = read_u64(input)?;
        checked_record_len(len, remaining)?;
        checked_body_len(len)?;
        discard_body(input, len)?;
        remaining -= len;
        blobs.push((hash, len));
    }

    if remaining != 0 {
        return Err(ArchiveError::TotalBytesMismatch {
            header: header.total_bytes,
            actual: header.total_bytes - remaining,
        });
    }
    reject_trailing_bytes(input)?;

    Ok(ArchiveContents {
        header,
        pinsets,
        blobs,
    })
}

fn collect_archive_blobs(
    store: &BlobStore,
    pinsets: &[Pinset],
    refs: &BlobReferencesRegistry,
) -> Result<BTreeSet<Hash256>, ArchiveError> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();

    for pinset in pinsets {
        for root in &pinset.roots {
            if seen.insert(*root) {
                queue.push_back(*root);
            }
        }
    }

    while let Some(hash) = queue.pop_front() {
        let bytes = store.get(hash)?;
        if let Some(children) = refs.referenced_blobs(hash, &bytes)? {
            for child in children {
                if seen.insert(child) {
                    queue.push_back(child);
                }
            }
        }
    }

    Ok(seen)
}

fn archive_blob_len(store: &BlobStore, hash: Hash256) -> Result<u64, ArchiveError> {
    match fs::metadata(store.path_for(hash)) {
        Ok(metadata) => Ok(metadata.len()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            Err(BlobStoreError::NotFound { hash }.into())
        }
        Err(err) => Err(err.into()),
    }
}

fn copy_verified_blob(
    store: &BlobStore,
    hash: Hash256,
    out: &mut dyn Write,
) -> Result<(), ArchiveError> {
    let mut input = store.get_streaming(hash)?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];

    loop {
        let read = input.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        out.write_all(&buf[..read])?;
    }

    let actual = Hash256::from_bytes(hasher.finalize().into());
    if actual != hash {
        return Err(ArchiveError::HashMismatch {
            expected: hash,
            actual,
        });
    }
    Ok(())
}

fn put_archive_body(store: &BlobStore, hash: Hash256, body: &[u8]) -> Result<(), ArchiveError> {
    store
        .put_expect(hash, body)
        .map(|_| ())
        .map_err(|err| match err {
            BlobStoreError::HashMismatch { expected, actual } => {
                ArchiveError::HashMismatch { expected, actual }
            }
            other => ArchiveError::BlobStore(other),
        })
}

fn validate_pinsets(pinsets: &[Pinset]) -> Result<(), ArchiveError> {
    if pinsets.len() > u16::MAX as usize {
        return Err(ArchiveError::TooManyPinsets {
            count: pinsets.len(),
            max: u16::MAX,
        });
    }
    for pinset in pinsets {
        let name_len = pinset.name.as_str().len();
        if name_len > u16::MAX as usize {
            return Err(ArchiveError::PinsetNameTooLong {
                len: name_len,
                max: u16::MAX,
            });
        }
        if let Some(annotation) = &pinset.annotation {
            let len = annotation.len();
            if len > u16::MAX as usize {
                return Err(ArchiveError::AnnotationTooLong { len, max: u16::MAX });
            }
        }
        checked_root_count(pinset.roots.len())?;
    }
    for pair in pinsets.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(ArchiveError::DuplicatePinsetName {
                name: pair[0].name.to_string(),
            });
        }
    }
    Ok(())
}

fn write_header(out: &mut dyn Write, header: &ArchiveHeader) -> Result<(), ArchiveError> {
    out.write_all(&header.magic)?;
    out.write_all(&[header.version])?;
    write_u16(out, header.pinset_count)?;
    out.write_all(&[0])?;
    write_u32(out, header.blob_count)?;
    write_u64(out, header.total_bytes)?;
    Ok(())
}

fn read_header(input: &mut dyn Read) -> Result<ArchiveHeader, ArchiveError> {
    let magic = read_array::<8>(input)?;
    if magic != ARCHIVE_MAGIC {
        return Err(ArchiveError::BadMagic { found: magic });
    }

    let version = read_u8(input)?;
    if version != ARCHIVE_VERSION {
        return Err(ArchiveError::UnsupportedVersion { found: version });
    }
    let pinset_count = read_u16(input)?;
    let reserved = read_u8(input)?;
    if reserved != 0 {
        return Err(ArchiveError::ReservedByteNonZero { found: reserved });
    }
    let blob_count = read_u32(input)?;
    let total_bytes = read_u64(input)?;

    Ok(ArchiveHeader {
        magic,
        version,
        pinset_count,
        blob_count,
        total_bytes,
    })
}

fn write_pinset(out: &mut dyn Write, pinset: &Pinset) -> Result<(), ArchiveError> {
    let name = pinset.name.as_str().as_bytes();
    write_u16(out, name.len() as u16)?;
    out.write_all(name)?;

    match &pinset.annotation {
        Some(annotation) => {
            out.write_all(&[1])?;
            write_u16(out, annotation.len() as u16)?;
            out.write_all(annotation.as_bytes())?;
        }
        None => out.write_all(&[0])?,
    }

    write_u32(out, checked_root_count(pinset.roots.len())?)?;
    for root in &pinset.roots {
        out.write_all(root.as_bytes())?;
    }
    Ok(())
}

fn read_pinsets(input: &mut dyn Read, count: u16) -> Result<Vec<Pinset>, ArchiveError> {
    let mut pinsets = Vec::with_capacity(count as usize);
    for _ in 0..count {
        pinsets.push(read_pinset(input)?);
    }
    Ok(pinsets)
}

fn read_pinset(input: &mut dyn Read) -> Result<Pinset, ArchiveError> {
    let name_len = read_u16(input)? as usize;
    let name_bytes = read_exact_vec(input, name_len)?;
    let name = str::from_utf8(&name_bytes)?;
    let name = PinsetName::new(name.to_owned())?;

    let annotation = match read_u8(input)? {
        0 => None,
        1 => {
            let len = read_u16(input)? as usize;
            let bytes = read_exact_vec(input, len)?;
            Some(str::from_utf8(&bytes)?.to_owned())
        }
        found => return Err(ArchiveError::InvalidAnnotationTag { found }),
    };

    let root_count = read_u32(input)?;
    let mut roots = BTreeSet::new();
    for _ in 0..root_count {
        roots.insert(read_hash(input)?);
    }

    Ok(Pinset {
        name,
        roots,
        annotation,
    })
}

fn read_hash(input: &mut dyn Read) -> Result<Hash256, ArchiveError> {
    Ok(Hash256::from_bytes(read_array::<32>(input)?))
}

fn read_body(input: &mut dyn Read, len: u64) -> Result<Vec<u8>, ArchiveError> {
    let len = checked_body_len(len)?;
    read_exact_vec(input, len)
}

fn discard_body(input: &mut dyn Read, mut len: u64) -> Result<(), ArchiveError> {
    let mut buf = [0_u8; 8 * 1024];
    while len > 0 {
        let chunk = usize::try_from(len.min(buf.len() as u64)).expect("chunk <= buf len");
        input
            .read_exact(&mut buf[..chunk])
            .map_err(map_read_error)?;
        len -= chunk as u64;
    }
    Ok(())
}

fn read_exact_vec(input: &mut dyn Read, len: usize) -> Result<Vec<u8>, ArchiveError> {
    let mut bytes = vec![0_u8; len];
    input.read_exact(&mut bytes).map_err(map_read_error)?;
    Ok(bytes)
}

fn reject_trailing_bytes(input: &mut dyn Read) -> Result<(), ArchiveError> {
    let mut byte = [0_u8; 1];
    loop {
        match input.read(&mut byte) {
            Ok(0) => return Ok(()),
            Ok(_) => return Err(ArchiveError::TrailingBytes),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(ArchiveError::Io(err)),
        }
    }
}

fn read_array<const N: usize>(input: &mut dyn Read) -> Result<[u8; N], ArchiveError> {
    let mut bytes = [0_u8; N];
    input.read_exact(&mut bytes).map_err(map_read_error)?;
    Ok(bytes)
}

fn read_u8(input: &mut dyn Read) -> Result<u8, ArchiveError> {
    Ok(read_array::<1>(input)?[0])
}

fn read_u16(input: &mut dyn Read) -> Result<u16, ArchiveError> {
    Ok(u16::from_le_bytes(read_array::<2>(input)?))
}

fn read_u32(input: &mut dyn Read) -> Result<u32, ArchiveError> {
    Ok(u32::from_le_bytes(read_array::<4>(input)?))
}

fn read_u64(input: &mut dyn Read) -> Result<u64, ArchiveError> {
    Ok(u64::from_le_bytes(read_array::<8>(input)?))
}

fn write_u16(out: &mut dyn Write, value: u16) -> Result<(), ArchiveError> {
    out.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_u32(out: &mut dyn Write, value: u32) -> Result<(), ArchiveError> {
    out.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_u64(out: &mut dyn Write, value: u64) -> Result<(), ArchiveError> {
    out.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn map_read_error(err: io::Error) -> ArchiveError {
    if err.kind() == io::ErrorKind::UnexpectedEof {
        ArchiveError::Truncated
    } else {
        ArchiveError::Io(err)
    }
}

#[derive(Debug)]
pub enum ArchiveError {
    Io(io::Error),
    BadMagic { found: [u8; 8] },
    UnsupportedVersion { found: u8 },
    ReservedByteNonZero { found: u8 },
    Truncated,
    Utf8(str::Utf8Error),
    BlobStore(BlobStoreError),
    BlobReferenceDecode(BlobReferenceError),
    PinsetName(PinsetNameError),
    TooManyPinsets { count: usize, max: u16 },
    TooManyBlobs { count: usize, max: u32 },
    PinsetNameTooLong { len: usize, max: u16 },
    DuplicatePinsetName { name: String },
    AnnotationTooLong { len: usize, max: u16 },
    TooManyRoots { count: usize, max: u32 },
    BlobTooLarge { len: u64, max: u64 },
    TotalBytesOverflow,
    TotalBytesMismatch { header: u64, actual: u64 },
    RecordExceedsDeclaredTotal { len: u64, remaining: u64 },
    HashMismatch { expected: Hash256, actual: Hash256 },
    InvalidAnnotationTag { found: u8 },
    TrailingBytes,
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "archive I/O error: {err}"),
            Self::BadMagic { found } => write!(f, "bad archive magic: {found:?}"),
            Self::UnsupportedVersion { found } => {
                write!(f, "unsupported archive version {found}")
            }
            Self::ReservedByteNonZero { found } => {
                write!(f, "archive reserved byte must be 0, got {found}")
            }
            Self::Truncated => f.write_str("archive input is truncated"),
            Self::Utf8(err) => write!(f, "archive UTF-8 error: {err}"),
            Self::BlobStore(err) => write!(f, "archive blob-store error: {err}"),
            Self::BlobReferenceDecode(err) => {
                write!(f, "archive blob-reference decode error: {err}")
            }
            Self::PinsetName(err) => write!(f, "archive pinset name error: {err}"),
            Self::TooManyPinsets { count, max } => {
                write!(f, "archive has {count} pinsets, maximum is {max}")
            }
            Self::TooManyBlobs { count, max } => {
                write!(f, "archive has {count} blobs, maximum is {max}")
            }
            Self::PinsetNameTooLong { len, max } => {
                write!(f, "pinset name length {len} exceeds maximum {max}")
            }
            Self::DuplicatePinsetName { name } => {
                write!(f, "duplicate archive pinset name {name}")
            }
            Self::AnnotationTooLong { len, max } => {
                write!(f, "pinset annotation length {len} exceeds maximum {max}")
            }
            Self::TooManyRoots { count, max } => {
                write!(f, "pinset has {count} roots, maximum is {max}")
            }
            Self::BlobTooLarge { len, max } => {
                write!(f, "archive blob length {len} exceeds maximum {max}")
            }
            Self::TotalBytesOverflow => f.write_str("archive total byte count overflowed"),
            Self::TotalBytesMismatch { header, actual } => {
                write!(
                    f,
                    "archive total bytes mismatch: header {header}, actual {actual}"
                )
            }
            Self::RecordExceedsDeclaredTotal { len, remaining } => {
                write!(
                    f,
                    "archive blob record length {len} exceeds remaining declared total {remaining}"
                )
            }
            Self::HashMismatch { expected, actual } => {
                write!(
                    f,
                    "archive hash mismatch: expected {expected}, got {actual}"
                )
            }
            Self::InvalidAnnotationTag { found } => {
                write!(f, "invalid annotation tag {found}")
            }
            Self::TrailingBytes => f.write_str("archive contains trailing bytes after records"),
        }
    }
}

impl std::error::Error for ArchiveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Utf8(err) => Some(err),
            Self::BlobStore(err) => Some(err),
            Self::BlobReferenceDecode(err) => Some(err),
            Self::PinsetName(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for ArchiveError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<str::Utf8Error> for ArchiveError {
    fn from(value: str::Utf8Error) -> Self {
        Self::Utf8(value)
    }
}

impl From<BlobStoreError> for ArchiveError {
    fn from(value: BlobStoreError) -> Self {
        Self::BlobStore(value)
    }
}

impl From<BlobReferenceError> for ArchiveError {
    fn from(value: BlobReferenceError) -> Self {
        Self::BlobReferenceDecode(value)
    }
}

impl From<PinsetNameError> for ArchiveError {
    fn from(value: PinsetNameError) -> Self {
        Self::PinsetName(value)
    }
}

#[doc(hidden)]
pub fn checked_root_count(count: usize) -> Result<u32, ArchiveError> {
    u32::try_from(count).map_err(|_| ArchiveError::TooManyRoots {
        count,
        max: u32::MAX,
    })
}

fn checked_record_len(len: u64, remaining: u64) -> Result<(), ArchiveError> {
    if len > remaining {
        return Err(ArchiveError::RecordExceedsDeclaredTotal { len, remaining });
    }
    Ok(())
}

fn checked_body_len(len: u64) -> Result<usize, ArchiveError> {
    if len > MAX_ARCHIVE_BLOB_BODY_LEN {
        return Err(ArchiveError::BlobTooLarge {
            len,
            max: MAX_ARCHIVE_BLOB_BODY_LEN,
        });
    }
    usize::try_from(len).map_err(|_| ArchiveError::BlobTooLarge {
        len,
        max: usize::MAX as u64,
    })
}
