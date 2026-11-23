//! A small reader for Java Edition's binary NBT (big endian), enough for `.litematic` files.

use std::{collections::BTreeMap, io::Read, path::Path};

#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<Tag>),
    Compound(BTreeMap<String, Tag>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(map) => map.get(key),
            _ => None,
        }
    }
    pub fn compound(&self) -> Option<&BTreeMap<String, Tag>> {
        match self {
            Tag::Compound(map) => Some(map),
            _ => None,
        }
    }
    pub fn list(&self) -> Option<&[Tag]> {
        match self {
            Tag::List(items) => Some(items),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }
    /// Any integer tag as i64.
    pub fn int(&self) -> Option<i64> {
        match *self {
            Tag::Byte(v) => Some(v.into()),
            Tag::Short(v) => Some(v.into()),
            Tag::Int(v) => Some(v.into()),
            Tag::Long(v) => Some(v),
            _ => None,
        }
    }
    pub fn longs(&self) -> Option<&[i64]> {
        match self {
            Tag::LongArray(v) => Some(v),
            _ => None,
        }
    }
    /// Takes `key` out of a compound (to keep a large array without copying it).
    pub fn remove(&mut self, key: &str) -> Option<Tag> {
        match self {
            Tag::Compound(map) => map.remove(key),
            _ => None,
        }
    }
}

const MAX_DEPTH: usize = 128;
const MAX_TAGS: usize = 4_000_000;
const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
const MAX_NBT_BYTES: usize = 512 * 1024 * 1024;
const INPUT_TOO_LARGE: &str = "schematic exceeds the import size limit";

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
    tags_left: usize,
    /// Long arrays are skipped (read as empty): enough to look at a file's metadata and sizes.
    skip_longs: bool,
}

type R<T> = Result<T, String>;

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or("NBT: length overflow")?;
        if end > self.data.len() {
            return Err("NBT: unexpected end of data".into());
        }
        let s = &self.data[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> R<i16> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u16(&mut self) -> R<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> R<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> R<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn array_len(&mut self) -> R<usize> {
        usize::try_from(self.i32()?).map_err(|_| "NBT: negative array length".into())
    }
    fn string(&mut self) -> R<String> {
        let n = self.u16()? as usize;
        crate::mutf8::decode(self.take(n)?)
    }
    fn payload(&mut self, kind: u8, depth: usize) -> R<Tag> {
        if depth > MAX_DEPTH {
            return Err("NBT: nested too deeply".into());
        }
        self.tags_left = self.tags_left.checked_sub(1).ok_or("NBT: too many tags")?;
        Ok(match kind {
            1 => Tag::Byte(self.u8()? as i8),
            2 => Tag::Short(self.i16()?),
            3 => Tag::Int(self.i32()?),
            4 => Tag::Long(self.i64()?),
            5 => Tag::Float(f32::from_bits(self.i32()? as u32)),
            6 => Tag::Double(f64::from_bits(self.i64()? as u64)),
            7 => {
                let n = self.array_len()?;
                let bytes = self.take(n)?;
                let mut v = vec_for(n)?;
                v.extend(bytes.iter().map(|&b| b as i8));
                Tag::ByteArray(v)
            }
            8 => Tag::String(self.string()?),
            9 => {
                let inner = self.u8()?;
                let n = self.i32()?.max(0) as usize;
                if n > self.tags_left {
                    return Err("NBT: too many tags".into());
                }
                let mut items = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    items.push(self.payload(inner, depth + 1)?);
                }
                Tag::List(items)
            }
            10 => {
                let mut map = BTreeMap::new();
                loop {
                    let kind = self.u8()?;
                    if kind == 0 {
                        break;
                    }
                    let name = self.string()?;
                    map.insert(name, self.payload(kind, depth + 1)?);
                }
                Tag::Compound(map)
            }
            11 => {
                let n = self.array_len()?;
                let bytes = self.take(n.checked_mul(4).ok_or("NBT: array too long")?)?;
                let mut v = vec_for(n)?;
                v.extend(bytes.chunks_exact(4).map(|c| i32::from_be_bytes(c.try_into().unwrap())));
                Tag::IntArray(v)
            }
            12 => {
                let n = self.array_len()?;
                let bytes = self.take(n.checked_mul(8).ok_or("NBT: array too long")?)?;
                if self.skip_longs {
                    Tag::LongArray(Vec::new())
                } else {
                    let mut v = vec_for(n)?;
                    v.extend(bytes.chunks_exact(8).map(|c| i64::from_be_bytes(c.try_into().unwrap())));
                    Tag::LongArray(v)
                }
            }
            k => return Err(format!("NBT: unknown tag type {k}")),
        })
    }
}

/// Reads an uncompressed named root tag; returns its name and value.
pub fn read(data: &[u8]) -> R<(String, Tag)> {
    read_with(data, false)
}

fn read_with(data: &[u8], skip_longs: bool) -> R<(String, Tag)> {
    if data.len() > MAX_NBT_BYTES {
        return Err(INPUT_TOO_LARGE.into());
    }
    let mut r = Reader { data, at: 0, tags_left: MAX_TAGS, skip_longs };
    let kind = r.u8()?;
    if kind == 0 {
        return Err("NBT: empty root".into());
    }
    let name = r.string()?;
    let tag = r.payload(kind, 0)?;
    Ok((name, tag))
}

/// The error a read gives when the system has no more memory for the file. Nothing else is
/// lost: what was taken is let go again.
pub const OUT_OF_MEMORY: &str = "not enough memory";

/// Deflate gives at most this many bytes per byte read (its densest form), so a gzip file's
/// footer is believed up to this many times the file's size.
const MAX_RATIO: usize = 1032;

/// Reads either gzip-compressed or raw NBT, with bounded input and decompression.
pub fn read_file_bytes(bytes: &[u8]) -> R<Tag> {
    read_file_with(bytes, false)
}

/// Like `read_file_bytes`, but every long array comes back empty: a large `.litematic`'s
/// metadata and region sizes without holding its blocks.
pub fn read_file_bytes_light(bytes: &[u8]) -> R<Tag> {
    read_file_with(bytes, true)
}

/// What a file of `len` bytes ending in `tail` unpacks to: a gzip file tells it in its footer
/// (modulo 4 GiB, and a file can claim anything, so it's only an estimate), any other file is
/// its own size.
pub fn unpacked_size(head: &[u8], tail: [u8; 4], len: u64) -> u64 {
    if head.starts_with(&[0x1f, 0x8b]) {
        u64::from(u32::from_le_bytes(tail)).min(len.saturating_mul(MAX_RATIO as u64))
    } else {
        len
    }
}

fn read_file_with(bytes: &[u8], skip_longs: bool) -> R<Tag> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(INPUT_TOO_LARGE.into());
    }
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let out = read_bounded(flate2::read::GzDecoder::new(bytes), MAX_NBT_BYTES)?;
        Ok(read_with(&out, skip_longs)?.1)
    } else {
        Ok(read_with(bytes, skip_longs)?.1)
    }
}

/// Reads a regular file under the same input cap used by the parser.
pub fn read_input(path: &Path) -> R<Vec<u8>> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("expected a regular schematic file".into());
    }
    if meta.len() > MAX_FILE_BYTES as u64 {
        return Err(INPUT_TOO_LARGE.into());
    }
    read_bounded(file, MAX_FILE_BYTES)
}

fn read_bounded(mut input: impl Read, limit: usize) -> R<Vec<u8>> {
    let mut buffer = [0; 32 * 1024];
    let mut out = Vec::new();
    loop {
        let want = buffer.len().min(limit - out.len() + 1);
        let n = match input.read(&mut buffer[..want]) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|e| e.to_string())?,
        };
        if n == 0 {
            return Ok(out);
        }
        let needed = out.len() + n;
        if needed > limit {
            return Err(INPUT_TOO_LARGE.into());
        }
        if needed > out.capacity() {
            let capacity = (out.capacity() * 2).max(needed).min(limit);
            out.try_reserve_exact(capacity - out.len()).map_err(|_| OUT_OF_MEMORY.to_string())?;
        }
        out.extend_from_slice(&buffer[..n]);
    }
}

/// A vector of `n` items, failing with [`OUT_OF_MEMORY`] where the system has no room.
fn vec_for<T>(n: usize) -> R<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| OUT_OF_MEMORY.to_string())?;
    Ok(v)
}

// ---- writer (used by tests to build files, and handy for exporting) ----

fn put_str(out: &mut Vec<u8>, s: &str) -> R<()> {
    let bytes = crate::mutf8::encode(s);
    let len = u16::try_from(bytes.len()).map_err(|_| "NBT: string exceeds 65535 encoded bytes")?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&bytes);
    Ok(())
}

fn kind(tag: &Tag) -> u8 {
    match tag {
        Tag::Byte(_) => 1,
        Tag::Short(_) => 2,
        Tag::Int(_) => 3,
        Tag::Long(_) => 4,
        Tag::Float(_) => 5,
        Tag::Double(_) => 6,
        Tag::ByteArray(_) => 7,
        Tag::String(_) => 8,
        Tag::List(_) => 9,
        Tag::Compound(_) => 10,
        Tag::IntArray(_) => 11,
        Tag::LongArray(_) => 12,
    }
}

fn put_payload(out: &mut Vec<u8>, tag: &Tag) -> R<()> {
    match tag {
        Tag::Byte(v) => out.push(*v as u8),
        Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Float(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
        Tag::Double(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
        Tag::ByteArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            out.extend(v.iter().map(|&b| b as u8));
        }
        Tag::String(s) => put_str(out, s)?,
        Tag::List(items) => {
            out.push(items.first().map_or(0, kind));
            out.extend_from_slice(&(items.len() as i32).to_be_bytes());
            for t in items {
                put_payload(out, t)?;
            }
        }
        Tag::Compound(map) => {
            for (k, v) in map {
                out.push(kind(v));
                put_str(out, k)?;
                put_payload(out, v)?;
            }
            out.push(0);
        }
        Tag::IntArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
        Tag::LongArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
    }
    Ok(())
}

/// Writes a named root tag (uncompressed).
pub fn write(name: &str, tag: &Tag) -> R<Vec<u8>> {
    let mut out = vec![kind(tag)];
    put_str(&mut out, name)?;
    put_payload(&mut out, tag)?;
    Ok(out)
}

/// Writes a gzip-compressed named root tag.
pub fn write_gzip(name: &str, tag: &Tag) -> R<Vec<u8>> {
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&write(name, tag)?).map_err(|e| e.to_string())?;
    enc.finish().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut map = BTreeMap::new();
        map.insert("a".to_string(), Tag::Int(-5));
        map.insert("s".to_string(), Tag::String("hällo".into()));
        map.insert("l".to_string(), Tag::LongArray(vec![1, -1, i64::MAX]));
        map.insert("list".to_string(), Tag::List(vec![Tag::Short(1), Tag::Short(2)]));
        let tag = Tag::Compound(map);
        let bytes = write_gzip("root", &tag).unwrap();
        assert_eq!(read_file_bytes(&bytes).unwrap(), tag);
    }

    /// A footer that lies is only a wrong hint for the memory taken: the file is unpacked and then
    /// refused (gzip checks the size), never a crash.
    #[test]
    fn footer_is_only_a_hint() {
        let tag = Tag::Compound(BTreeMap::from([("l".into(), Tag::LongArray((0..300_000).collect()))]));
        let bytes = write_gzip("", &tag).unwrap();
        let at = bytes.len() - 4;
        let real = u32::from_le_bytes(bytes[at..].try_into().unwrap());
        assert_eq!(unpacked_size(&bytes, bytes[at..].try_into().unwrap(), bytes.len() as u64), u64::from(real));
        let gunzip = |bytes: &[u8]| read_bounded(flate2::read::GzDecoder::new(bytes), MAX_NBT_BYTES);
        assert_eq!(gunzip(&bytes).unwrap().len(), real as usize);
        for claim in [0, 1, real - 1, real + 1, u32::MAX] {
            let mut lying = bytes.clone();
            lying[at..].copy_from_slice(&claim.to_le_bytes());
            let result = gunzip(&lying);
            assert!(result.as_ref().is_err_and(|e| e != OUT_OF_MEMORY), "claim {claim}: {result:?}");
        }
        // Believed only up to deflate's densest ratio; plain NBT is its own size.
        assert_eq!(unpacked_size(&[0x1f, 0x8b], u32::MAX.to_le_bytes(), 10), 10 * MAX_RATIO as u64);
        assert_eq!(unpacked_size(&[10, 0], u32::MAX.to_le_bytes(), 10), 10);
    }

    #[test]
    fn truncated_is_an_error() {
        let bytes = write("", &Tag::Compound(BTreeMap::from([("x".into(), Tag::Long(3))]))).unwrap();
        assert!(read(&bytes[..bytes.len() - 3]).is_err());
    }

    #[test]
    fn java_strings_round_trip_at_the_nbt_boundary() {
        let tag = Tag::Compound(BTreeMap::from([("💎".into(), Tag::String("A\0😀".into()))]));
        let bytes = write("", &tag).unwrap();
        let java_bytes = [10, 0, 0, 8, 0, 6, 0xed, 0xa0, 0xbd, 0xed, 0xb2, 0x8e,
            0, 9, b'A', 0xc0, 0x80, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80, 0];
        assert_eq!(bytes, java_bytes);
        assert_eq!(read(&java_bytes).unwrap().1, tag);
        assert!(write("", &Tag::String("😀".repeat(12_000))).is_err());
    }

    #[test]
    fn import_limits_are_enforced() {
        let bytes = write_gzip("", &Tag::ByteArray(vec![0; 1024])).unwrap();
        assert_eq!(read_bounded(flate2::read::GzDecoder::new(&bytes[..]), 1024), Err(INPUT_TOO_LARGE.into()));
        assert_eq!(read_bounded(&b"abc"[..], 3).unwrap(), b"abc");
        assert!(read(&[7, 0, 0, 255, 255, 255, 255]).unwrap_err().contains("negative array"));
        let n = ((MAX_TAGS + 1) as i32).to_be_bytes();
        assert!(read(&[9, 0, 0, 1, n[0], n[1], n[2], n[3]]).unwrap_err().contains("too many tags"));
    }
}
