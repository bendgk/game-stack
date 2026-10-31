//! A minimal zip reader (stored + deflate): Minecraft jars, resource packs and zipped worlds.

use std::collections::BTreeMap;
use std::io::Read;

pub struct Zip {
    data: Vec<u8>,
    /// name → (offset of the local header, compressed size, method, uncompressed size)
    entries: BTreeMap<String, (usize, usize, u16, usize)>,
}

fn u16le(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn u32le(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

impl Zip {
    pub fn new(data: Vec<u8>) -> Result<Zip, String> {
        // End of central directory: scan back over the comment.
        let n = data.len();
        let start = n.saturating_sub(22 + 65535);
        let eocd = (start..n.saturating_sub(21))
            .rev()
            .find(|&i| u32le(&data, i) == Some(0x0605_4b50))
            .ok_or("not a zip file")?;
        let count = u16le(&data, eocd + 10).ok_or("bad zip")? as usize;
        let mut at = u32le(&data, eocd + 16).ok_or("bad zip")? as usize;
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            if u32le(&data, at) != Some(0x0201_4b50) {
                return Err("bad zip central directory".into());
            }
            let method = u16le(&data, at + 10).ok_or("bad zip")?;
            let csize = u32le(&data, at + 20).ok_or("bad zip")? as usize;
            let usize_ = u32le(&data, at + 24).ok_or("bad zip")? as usize;
            let nlen = u16le(&data, at + 28).ok_or("bad zip")? as usize;
            let xlen = u16le(&data, at + 30).ok_or("bad zip")? as usize;
            let clen = u16le(&data, at + 32).ok_or("bad zip")? as usize;
            let local = u32le(&data, at + 42).ok_or("bad zip")? as usize;
            let name = data.get(at + 46..at + 46 + nlen).ok_or("bad zip")?;
            entries.insert(String::from_utf8_lossy(name).replace('\\', "/"), (local, csize, method, usize_));
            at += 46 + nlen + xlen + clen;
        }
        Ok(Zip { data, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        let &(local, csize, method, size) = self.entries.get(name)?;
        if u32le(&self.data, local)? != 0x0403_4b50 || size > 512 << 20 {
            return None;
        }
        let nlen = u16le(&self.data, local + 26)? as usize;
        let xlen = u16le(&self.data, local + 28)? as usize;
        let body = self.data.get(local + 30 + nlen + xlen..local + 30 + nlen + xlen + csize)?;
        match method {
            0 => Some(body.to_vec()),
            8 => {
                let mut out = Vec::with_capacity(size);
                flate2::read::DeflateDecoder::new(body).take(size as u64 + 1).read_to_end(&mut out).ok()?;
                Some(out)
            }
            _ => None,
        }
    }
}
