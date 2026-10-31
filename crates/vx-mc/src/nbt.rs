//! Named Binary Tag (Minecraft's save format): just enough to read worlds. Big-endian,
//! hostile-input safe (bounds-checked, depth-limited).

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    End,
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Bytes(Vec<i8>),
    String(String),
    List(Vec<Tag>),
    Compound(BTreeMap<String, Tag>),
    Ints(Vec<i32>),
    Longs(Vec<i64>),
}

impl Tag {
    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(m) => m.get(key),
            _ => None,
        }
    }
    /// Follow a path of compound keys: `t.at(&["Data", "SpawnX"])`.
    pub fn at(&self, path: &[&str]) -> Option<&Tag> {
        path.iter().try_fold(self, |t, k| t.get(k))
    }
    pub fn int(&self) -> Option<i64> {
        match self {
            Tag::Byte(v) => Some(*v as i64),
            Tag::Short(v) => Some(*v as i64),
            Tag::Int(v) => Some(*v as i64),
            Tag::Long(v) => Some(*v),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn list(&self) -> &[Tag] {
        match self {
            Tag::List(v) => v,
            _ => &[],
        }
    }
    pub fn longs(&self) -> &[i64] {
        match self {
            Tag::Longs(v) => v,
            _ => &[],
        }
    }
    pub fn ints(&self) -> Option<&[i32]> {
        match self {
            Tag::Ints(v) => Some(v),
            _ => None,
        }
    }
    pub fn compound(&self) -> Option<&BTreeMap<String, Tag>> {
        match self {
            Tag::Compound(m) => Some(m),
            _ => None,
        }
    }
}

struct R<'a> {
    b: &'a [u8],
    i: usize,
}

impl R<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.i.checked_add(n).filter(|e| *e <= self.b.len()).ok_or("truncated NBT")?;
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> Result<i16, String> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn len(&mut self) -> Result<usize, String> {
        let n = self.i32()?;
        if n < 0 || n as usize > self.b.len() {
            return Err("bad NBT length".into());
        }
        Ok(n as usize)
    }
    fn string(&mut self) -> Result<String, String> {
        let n = u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as usize;
        // Modified UTF-8; lossy is fine for names.
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn payload(&mut self, kind: u8, depth: u32) -> Result<Tag, String> {
        if depth > 64 {
            return Err("NBT nested too deep".into());
        }
        Ok(match kind {
            0 => Tag::End,
            1 => Tag::Byte(self.u8()? as i8),
            2 => Tag::Short(self.i16()?),
            3 => Tag::Int(self.i32()?),
            4 => Tag::Long(self.i64()?),
            5 => Tag::Float(f32::from_bits(self.i32()? as u32)),
            6 => Tag::Double(f64::from_bits(self.i64()? as u64)),
            7 => {
                let n = self.len()?;
                Tag::Bytes(self.take(n)?.iter().map(|b| *b as i8).collect())
            }
            8 => Tag::String(self.string()?),
            9 => {
                let k = self.u8()?;
                let n = self.len()?;
                let mut v = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    v.push(self.payload(k, depth + 1)?);
                }
                Tag::List(v)
            }
            10 => {
                let mut m = BTreeMap::new();
                loop {
                    let k = self.u8()?;
                    if k == 0 {
                        break;
                    }
                    let name = self.string()?;
                    m.insert(name, self.payload(k, depth + 1)?);
                }
                Tag::Compound(m)
            }
            11 => {
                let n = self.len()?;
                let mut v = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    v.push(self.i32()?);
                }
                Tag::Ints(v)
            }
            12 => {
                let n = self.len()?;
                let mut v = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    v.push(self.i64()?);
                }
                Tag::Longs(v)
            }
            k => return Err(format!("unknown NBT tag {k}")),
        })
    }
}

/// Parse an (uncompressed) NBT document: the root compound.
pub fn parse(b: &[u8]) -> Result<Tag, String> {
    let mut r = R { b, i: 0 };
    let kind = r.u8()?;
    if kind != 10 {
        return Err("NBT root is not a compound".into());
    }
    let _name = r.string()?;
    r.payload(10, 0)
}

/// Gzip-compressed NBT (level.dat).
pub fn parse_gz(b: &[u8]) -> Result<Tag, String> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(b).take(64 << 20).read_to_end(&mut out).map_err(|e| e.to_string())?;
    parse(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_small_document_and_rejects_garbage() {
        // { "hi": { "n": Int 7, "s": "ok" } }
        let mut b = vec![10, 0, 2, b'h', b'i'];
        b.extend([3, 0, 1, b'n', 0, 0, 0, 7]);
        b.extend([8, 0, 1, b's', 0, 2, b'o', b'k']);
        b.push(0);
        let t = parse(&b).unwrap();
        assert_eq!(t.at(&["n"]).and_then(Tag::int), Some(7));
        assert_eq!(t.at(&["s"]).and_then(Tag::str), Some("ok"));
        assert!(parse(&b[..6]).is_err());
        assert!(parse(&[10, 0, 0, 9, 0, 1, b'l', 3, 0x7f, 0xff, 0xff, 0xff]).is_err(), "huge list length");
    }
}
