//! Game-agnostic snapshot delta compression.
//!
//! XOR the new encoding against an acknowledged baseline, then run-length encode the zero
//! runs. Unchanged entities XOR to zeros, so a mostly-static world costs a few bytes. It never
//! needs to understand the game's types — only that both sides hold the same baseline bytes.
//!
//! Format: `varint len`, then repeated `(varint zeros, varint literals, literal bytes…)` until
//! `len` bytes are produced. Decoding is fully bounds-checked.

use gs_wire::{Reader, WireError, Writer};

pub fn encode(base: &[u8], cur: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(cur.len() as u64);
    let x = |i: usize| cur[i] ^ base.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while i < cur.len() {
        let z0 = i;
        while i < cur.len() && x(i) == 0 {
            i += 1;
        }
        let zeros = i - z0;
        let l0 = i;
        // A literal run ends at the first run of ≥3 zeros (shorter zero runs are cheaper inline).
        while i < cur.len() {
            if x(i) == 0 && (i + 2 >= cur.len() || (x(i + 1) == 0 && x(i + 2) == 0)) {
                break;
            }
            i += 1;
        }
        w.varint(zeros as u64);
        w.varint((i - l0) as u64);
        for k in l0..i {
            w.u8(x(k));
        }
    }
    w.finish()
}

pub fn decode(base: &[u8], delta: &[u8]) -> Result<Vec<u8>, WireError> {
    let mut r = Reader::new(delta);
    let len = r.varint()? as usize;
    if len > gs_wire::MAX_SEQ * 16 {
        return Err(WireError::TooLong(len as u64));
    }
    let mut out = Vec::with_capacity(len);
    let b = |i: usize| base.get(i).copied().unwrap_or(0);
    while out.len() < len {
        let zeros = r.varint()? as usize;
        let lits = r.varint()? as usize;
        if zeros.saturating_add(lits) > len - out.len() {
            return Err(WireError::TooLong((zeros + lits) as u64));
        }
        for _ in 0..zeros {
            out.push(b(out.len()));
        }
        for &x in r.take(lits)? {
            out.push(x ^ b(out.len()));
        }
        if zeros == 0 && lits == 0 {
            return Err(WireError::BadTag(0));
        }
    }
    if r.remaining() != 0 {
        return Err(WireError::Trailing(r.remaining()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(base: &[u8], cur: &[u8]) -> usize {
        let d = encode(base, cur);
        assert_eq!(decode(base, &d).unwrap(), cur);
        d.len()
    }

    #[test]
    fn roundtrip_cases() {
        rt(&[], &[]);
        rt(&[], &[1, 2, 3]);
        rt(&[1, 2, 3], &[]);
        rt(&[1, 2, 3, 4], &[1, 2, 3, 4]);
        rt(&[1, 2, 3, 4, 5, 6], &[1, 9, 3, 4, 5, 6, 7, 8]);
        let base: Vec<u8> = (0..1000).map(|i| (i * 7) as u8).collect();
        let mut cur = base.clone();
        cur[500] ^= 1;
        cur[900] ^= 0xff;
        assert!(rt(&base, &cur) < 16, "sparse change compresses");
    }

    #[test]
    fn fuzz_roundtrip_and_garbage() {
        let mut s = 99u64;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..2000 {
            let n = (rnd() % 64) as usize;
            let base: Vec<u8> = (0..n).map(|_| rnd() as u8 & 3).collect();
            let m = (rnd() % 64) as usize;
            let cur: Vec<u8> = (0..m).map(|_| rnd() as u8 & 3).collect();
            rt(&base, &cur);
            let garbage: Vec<u8> = (0..(rnd() % 20)).map(|_| rnd() as u8).collect();
            let _ = decode(&base, &garbage);
        }
    }
}
