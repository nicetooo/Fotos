//! Finding the camera's own JPEG preview inside a RAW file.
//!
//! Most RAW formats (NEF, CR2, ARW, DNG, PEF, ORF, RW2…) are TIFF files. The
//! camera stores one or more JPEG renderings in them and records where each
//! one is in the TIFF directories: a thumbnail of about 160 px, and a preview
//! that is usually the full picture. Reading those records finds the big one
//! directly; searching the file for JPEG signatures tends to stop at the
//! thumbnail.

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};

const TAG_COMPRESSION: u16 = 0x0103;
const TAG_STRIP_OFFSETS: u16 = 0x0111;
const TAG_STRIP_BYTE_COUNTS: u16 = 0x0117;
const TAG_SUB_IFDS: u16 = 0x014A;
const TAG_JPEG_OFFSET: u16 = 0x0201;
const TAG_JPEG_LENGTH: u16 = 0x0202;

/// More directories than any real file has; a bound against corrupt loops.
const MAX_DIRECTORIES: usize = 64;

struct Tiff<R> {
    reader: R,
    little_endian: bool,
}

impl<R: Read + Seek> Tiff<R> {
    fn u16_at(&mut self, offset: u64) -> Option<u16> {
        let mut b = [0u8; 2];
        self.reader.seek(SeekFrom::Start(offset)).ok()?;
        self.reader.read_exact(&mut b).ok()?;
        Some(if self.little_endian { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32_at(&mut self, offset: u64) -> Option<u32> {
        let mut b = [0u8; 4];
        self.reader.seek(SeekFrom::Start(offset)).ok()?;
        self.reader.read_exact(&mut b).ok()?;
        Some(if self.little_endian { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }
}

/// `(offset, length)` of every JPEG stream the TIFF directories of a RAW file
/// point to, in the order found. Empty when the file is not TIFF-based.
pub fn find_embedded_jpegs<R: Read + Seek>(reader: R) -> Vec<(u64, u64)> {
    let mut tiff = Tiff { reader, little_endian: true };
    let mut head = [0u8; 2];
    if tiff.reader.seek(SeekFrom::Start(0)).is_err() || tiff.reader.read_exact(&mut head).is_err() {
        return Vec::new();
    }
    tiff.little_endian = match &head {
        b"II" => true,
        b"MM" => false,
        _ => return Vec::new(),
    };
    let Some(first) = tiff.u32_at(4) else { return Vec::new() };

    let mut found = Vec::new();
    let mut queue = vec![first as u64];
    let mut seen = HashSet::new();
    while let Some(dir) = queue.pop() {
        if dir == 0 || !seen.insert(dir) || seen.len() > MAX_DIRECTORIES {
            continue;
        }
        let Some(count) = tiff.u16_at(dir) else { continue };
        let (mut jpeg_offset, mut jpeg_length) = (None, None);
        let (mut strip_offset, mut strip_length, mut compression) = (None, None, 0u32);
        for i in 0..count as u64 {
            let entry = dir + 2 + i * 12;
            let (Some(tag), Some(kind), Some(n), Some(value)) = (
                tiff.u16_at(entry),
                tiff.u16_at(entry + 2),
                tiff.u32_at(entry + 4),
                tiff.u32_at(entry + 8),
            ) else {
                break;
            };
            // A SHORT value sits in the first two bytes of the value field.
            let scalar = if kind == 3 { tiff.u16_at(entry + 8).map(u32::from).unwrap_or(0) } else { value };
            match tag {
                TAG_JPEG_OFFSET => jpeg_offset = Some(scalar),
                TAG_JPEG_LENGTH => jpeg_length = Some(scalar),
                TAG_COMPRESSION => compression = scalar,
                TAG_STRIP_OFFSETS if n == 1 => strip_offset = Some(scalar),
                TAG_STRIP_BYTE_COUNTS if n == 1 => strip_length = Some(scalar),
                TAG_SUB_IFDS => {
                    if n == 1 {
                        queue.push(value as u64);
                    } else {
                        for k in 0..n.min(16) as u64 {
                            if let Some(sub) = tiff.u32_at(value as u64 + k * 4) {
                                queue.push(sub as u64);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if let (Some(offset), Some(length)) = (jpeg_offset, jpeg_length) {
            found.push((offset as u64, length as u64));
        }
        // "Old-style" (6) or regular (7) JPEG stored as a single strip.
        if let (Some(offset), Some(length), 6 | 7) = (strip_offset, strip_length, compression) {
            found.push((offset as u64, length as u64));
        }
        if let Some(next) = tiff.u32_at(dir + 2 + count as u64 * 12) {
            queue.push(next as u64);
        }
    }
    found
}

/// The largest JPEG stream the directories point to, read into memory.
/// `None` when there is none or it does not start like a JPEG.
pub fn largest_embedded_jpeg<R: Read + Seek>(mut reader: R, file_size: u64) -> Option<Vec<u8>> {
    let mut candidates = find_embedded_jpegs(&mut reader);
    candidates.sort_by(|a, b| b.1.cmp(&a.1));
    for (offset, length) in candidates {
        if length < 4 || offset.checked_add(length)? > file_size {
            continue;
        }
        let mut data = vec![0u8; length as usize];
        if reader.seek(SeekFrom::Start(offset)).is_err() || reader.read_exact(&mut data).is_err() {
            continue;
        }
        if data[0] == 0xFF && data[1] == 0xD8 {
            return Some(data);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A little-endian TIFF: IFD0 points to a small JPEG and, through SubIFDs,
    /// to a directory with a bigger one.
    fn sample() -> Vec<u8> {
        let mut f = vec![0u8; 400];
        f[0..4].copy_from_slice(b"II\x2a\x00");
        f[4..8].copy_from_slice(&8u32.to_le_bytes());
        let mut entry = |f: &mut Vec<u8>, at: usize, tag: u16, value: u32| {
            f[at..at + 2].copy_from_slice(&tag.to_le_bytes());
            f[at + 2..at + 4].copy_from_slice(&4u16.to_le_bytes());
            f[at + 4..at + 8].copy_from_slice(&1u32.to_le_bytes());
            f[at + 8..at + 12].copy_from_slice(&value.to_le_bytes());
        };
        // IFD0 at 8: three entries.
        f[8..10].copy_from_slice(&3u16.to_le_bytes());
        entry(&mut f, 10, TAG_SUB_IFDS, 100);
        entry(&mut f, 22, TAG_JPEG_OFFSET, 200);
        entry(&mut f, 34, TAG_JPEG_LENGTH, 10);
        // Sub directory at 100: two entries.
        f[100..102].copy_from_slice(&2u16.to_le_bytes());
        entry(&mut f, 102, TAG_JPEG_OFFSET, 300);
        entry(&mut f, 114, TAG_JPEG_LENGTH, 60);
        f[200..202].copy_from_slice(&[0xFF, 0xD8]);
        f[300..302].copy_from_slice(&[0xFF, 0xD8]);
        f[359] = 0xD9;
        f
    }

    #[test]
    fn finds_the_preview_in_a_sub_directory() {
        let file = sample();
        let mut found = find_embedded_jpegs(Cursor::new(&file));
        found.sort();
        assert_eq!(found, vec![(200, 10), (300, 60)]);
        let big = largest_embedded_jpeg(Cursor::new(&file), file.len() as u64).unwrap();
        assert_eq!(big.len(), 60);
        assert_eq!(big[59], 0xD9);
    }

    #[test]
    fn ignores_files_that_are_not_tiff() {
        assert!(find_embedded_jpegs(Cursor::new(b"\xFF\xD8\xFF\xE0 not a tiff".to_vec())).is_empty());
        assert!(find_embedded_jpegs(Cursor::new(Vec::<u8>::new())).is_empty());
    }
}
