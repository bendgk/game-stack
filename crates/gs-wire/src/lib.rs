//! # gs-wire — the wire contract
//!
//! A small Cap'n-Proto-spirited binary codec built for game netcode:
//!
//! - **One source of truth.** Every message is a Rust type with `#[derive(Wire)]`. The server
//!   (native) and the browser (wasm32) compile the *same* type, so there is no second schema
//!   to drift from.
//! - **Fingerprinted.** [`Wire::schema`] writes a canonical description of the type (names,
//!   order, field types, recursively). [`fingerprint`] hashes it; the handshake refuses a peer
//!   whose fingerprint differs, and `contract/schema.txt` makes every protocol change visible
//!   in review.
//! - **Hostile-input safe.** Decoding never panics, never over-allocates (lengths are checked
//!   against the bytes actually left), caps nesting depth and rejects trailing bytes.
//! - **Compact and stable.** Integers are LEB128 varints (zig-zag for signed), floats are raw
//!   little-endian IEEE bits (exact round-trip, which determinism needs), enums are a varint tag.
//!
//! ```
//! use gs_wire::{Wire, from_bytes, to_bytes};
//! #[derive(Wire, Debug, PartialEq)]
//! struct Move { dx: f32, dy: f32, jump: bool }
//! let bytes = to_bytes(&Move { dx: 1.0, dy: -0.5, jump: true });
//! assert_eq!(from_bytes::<Move>(&bytes).unwrap(), Move { dx: 1.0, dy: -0.5, jump: true });
//! ```

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

extern crate self as gs_wire;
pub use gs_wire_derive::Wire;

mod schema;
pub use schema::{Schema, describe, fingerprint, fnv1a64, short_type_name};

/// Hard ceiling on any single sequence length (elements or bytes).
pub const MAX_SEQ: usize = 1 << 20;
/// Maximum nesting depth of derived types while decoding (stack-overflow guard).
pub const MAX_DEPTH: u32 = 64;

/// Why a decode failed. Decoding hostile bytes returns one of these — it never panics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    Eof,
    BadTag(u64),
    VarintOverflow,
    TooLong(u64),
    Utf8,
    TooDeep,
    Trailing(usize),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Eof => write!(f, "unexpected end of message"),
            WireError::BadTag(t) => write!(f, "unknown enum tag {t}"),
            WireError::VarintOverflow => write!(f, "varint overflow"),
            WireError::TooLong(n) => write!(f, "length {n} exceeds remaining bytes / limit"),
            WireError::Utf8 => write!(f, "invalid utf-8"),
            WireError::TooDeep => write!(f, "nesting deeper than {MAX_DEPTH}"),
            WireError::Trailing(n) => write!(f, "{n} trailing bytes after message"),
        }
    }
}

impl std::error::Error for WireError {}

/// A type that can cross the wire. Derive it; implement by hand only for leaf types.
pub trait Wire: Sized {
    fn encode(&self, w: &mut Writer);
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError>;
    /// Write this type's canonical schema (see [`Schema`]).
    fn schema(s: &mut Schema);
}

/// Encode a value into a fresh buffer.
pub fn to_bytes<T: Wire>(v: &T) -> Vec<u8> {
    let mut w = Writer::new();
    v.encode(&mut w);
    w.finish()
}

/// Decode a whole buffer; trailing bytes are an error.
pub fn from_bytes<T: Wire>(bytes: &[u8]) -> Result<T, WireError> {
    let mut r = Reader::new(bytes);
    let v = T::decode(&mut r)?;
    match r.remaining() {
        0 => Ok(v),
        n => Err(WireError::Trailing(n)),
    }
}

// ---------------------------------------------------------------------------------------------
// Writer / Reader
// ---------------------------------------------------------------------------------------------

#[derive(Default, Debug, Clone)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { buf: Vec::with_capacity(64) }
    }
    /// Reuse an existing allocation (cleared).
    pub fn reuse(mut buf: Vec<u8>) -> Self {
        buf.clear();
        Self { buf }
    }
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
    #[inline]
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    #[inline]
    pub fn bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }
    #[inline]
    pub fn varint(&mut self, mut v: u64) {
        while v >= 0x80 {
            self.buf.push((v as u8) | 0x80);
            v >>= 7;
        }
        self.buf.push(v as u8);
    }
    #[inline]
    pub fn zigzag(&mut self, v: i64) {
        self.varint(((v << 1) ^ (v >> 63)) as u64);
    }
}

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    depth: u32,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0, depth: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }
    pub fn position(&self) -> usize {
        self.pos
    }
    #[inline]
    pub fn u8(&mut self) -> Result<u8, WireError> {
        let b = *self.buf.get(self.pos).ok_or(WireError::Eof)?;
        self.pos += 1;
        Ok(b)
    }
    #[inline]
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        if n > self.remaining() {
            return Err(WireError::Eof);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    #[inline]
    pub fn varint(&mut self) -> Result<u64, WireError> {
        let mut out = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            // The 10th byte may only carry the top bit of a u64.
            if shift == 63 && b > 1 {
                return Err(WireError::VarintOverflow);
            }
            out |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(out);
            }
        }
        Err(WireError::VarintOverflow)
    }
    #[inline]
    pub fn zigzag(&mut self) -> Result<i64, WireError> {
        let v = self.varint()?;
        Ok(((v >> 1) as i64) ^ -((v & 1) as i64))
    }
    /// A sequence length, validated against the hard cap and (for element types that occupy at
    /// least one byte) the bytes actually left — so a forged length can't trigger a huge alloc.
    #[inline]
    pub fn len(&mut self, min_elem_bytes: usize) -> Result<usize, WireError> {
        let n = self.varint()?;
        let budget = self.remaining().checked_div(min_elem_bytes).unwrap_or(MAX_SEQ);
        if n as usize > MAX_SEQ || n as usize > budget {
            return Err(WireError::TooLong(n));
        }
        Ok(n as usize)
    }
    pub fn enter(&mut self) -> Result<(), WireError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { Err(WireError::TooDeep) } else { Ok(()) }
    }
    pub fn leave(&mut self) {
        self.depth -= 1;
    }
}

// ---------------------------------------------------------------------------------------------
// Leaf impls
// ---------------------------------------------------------------------------------------------

macro_rules! uint {
    ($($t:ty),*) => {$(
        impl Wire for $t {
            #[inline] fn encode(&self, w: &mut Writer) { w.varint(*self as u64) }
            #[inline] fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
                let v = r.varint()?;
                <$t>::try_from(v).map_err(|_| WireError::VarintOverflow)
            }
            fn schema(s: &mut Schema) { s.leaf(stringify!($t)) }
        }
    )*};
}
uint!(u16, u32, u64);

macro_rules! sint {
    ($($t:ty),*) => {$(
        impl Wire for $t {
            #[inline] fn encode(&self, w: &mut Writer) { w.zigzag(*self as i64) }
            #[inline] fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
                let v = r.zigzag()?;
                <$t>::try_from(v).map_err(|_| WireError::VarintOverflow)
            }
            fn schema(s: &mut Schema) { s.leaf(stringify!($t)) }
        }
    )*};
}
sint!(i16, i32, i64);

impl Wire for u8 {
    #[inline]
    fn encode(&self, w: &mut Writer) {
        w.u8(*self)
    }
    #[inline]
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        r.u8()
    }
    fn schema(s: &mut Schema) {
        s.leaf("u8")
    }
}

impl Wire for i8 {
    #[inline]
    fn encode(&self, w: &mut Writer) {
        w.u8(*self as u8)
    }
    #[inline]
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        Ok(r.u8()? as i8)
    }
    fn schema(s: &mut Schema) {
        s.leaf("i8")
    }
}

impl Wire for bool {
    #[inline]
    fn encode(&self, w: &mut Writer) {
        w.u8(*self as u8)
    }
    #[inline]
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        match r.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            t => Err(WireError::BadTag(t as u64)),
        }
    }
    fn schema(s: &mut Schema) {
        s.leaf("bool")
    }
}

impl Wire for f32 {
    #[inline]
    fn encode(&self, w: &mut Writer) {
        w.bytes(&self.to_bits().to_le_bytes())
    }
    #[inline]
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let b = r.take(4)?;
        Ok(f32::from_bits(u32::from_le_bytes([b[0], b[1], b[2], b[3]])))
    }
    fn schema(s: &mut Schema) {
        s.leaf("f32")
    }
}

impl Wire for f64 {
    #[inline]
    fn encode(&self, w: &mut Writer) {
        w.bytes(&self.to_bits().to_le_bytes())
    }
    #[inline]
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let b = r.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(f64::from_bits(u64::from_le_bytes(a)))
    }
    fn schema(s: &mut Schema) {
        s.leaf("f64")
    }
}

impl Wire for () {
    fn encode(&self, _: &mut Writer) {}
    fn decode(_: &mut Reader<'_>) -> Result<Self, WireError> {
        Ok(())
    }
    fn schema(s: &mut Schema) {
        s.leaf("()")
    }
}

impl Wire for String {
    fn encode(&self, w: &mut Writer) {
        w.varint(self.len() as u64);
        w.bytes(self.as_bytes());
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let n = r.len(1)?;
        let b = r.take(n)?;
        std::str::from_utf8(b).map(str::to_owned).map_err(|_| WireError::Utf8)
    }
    fn schema(s: &mut Schema) {
        s.leaf("string")
    }
}

/// Raw bytes with a memcpy fast path (use instead of `Vec<u8>` for opaque payloads).
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Blob(pub Vec<u8>);

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Blob({} bytes)", self.0.len())
    }
}

impl Wire for Blob {
    fn encode(&self, w: &mut Writer) {
        w.varint(self.0.len() as u64);
        w.bytes(&self.0);
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let n = r.len(1)?;
        Ok(Blob(r.take(n)?.to_vec()))
    }
    fn schema(s: &mut Schema) {
        s.leaf("blob")
    }
}

// ---------------------------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------------------------

fn min_bytes<T>() -> usize {
    // Every non-zero-sized wire type encodes to at least one byte.
    usize::from(std::mem::size_of::<T>() != 0)
}

impl<T: Wire> Wire for Vec<T> {
    fn encode(&self, w: &mut Writer) {
        w.varint(self.len() as u64);
        for v in self {
            v.encode(w);
        }
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let n = r.len(min_bytes::<T>())?;
        let mut out = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            out.push(T::decode(r)?);
        }
        Ok(out)
    }
    fn schema(s: &mut Schema) {
        s.wrap("vec", T::schema)
    }
}

impl<T: Wire> Wire for VecDeque<T> {
    fn encode(&self, w: &mut Writer) {
        w.varint(self.len() as u64);
        for v in self {
            v.encode(w);
        }
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        Ok(Vec::<T>::decode(r)?.into())
    }
    fn schema(s: &mut Schema) {
        s.wrap("vec", T::schema)
    }
}

impl<T: Wire> Wire for Option<T> {
    fn encode(&self, w: &mut Writer) {
        match self {
            None => w.u8(0),
            Some(v) => {
                w.u8(1);
                v.encode(w)
            }
        }
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        match r.u8()? {
            0 => Ok(None),
            1 => Ok(Some(T::decode(r)?)),
            t => Err(WireError::BadTag(t as u64)),
        }
    }
    fn schema(s: &mut Schema) {
        s.wrap("option", T::schema)
    }
}

impl<T: Wire> Wire for Box<T> {
    fn encode(&self, w: &mut Writer) {
        (**self).encode(w)
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        r.enter()?;
        let v = T::decode(r)?;
        r.leave();
        Ok(Box::new(v))
    }
    fn schema(s: &mut Schema) {
        T::schema(s)
    }
}

impl<T: Wire, const N: usize> Wire for [T; N] {
    fn encode(&self, w: &mut Writer) {
        for v in self {
            v.encode(w);
        }
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut v = Vec::with_capacity(N);
        for _ in 0..N {
            v.push(T::decode(r)?);
        }
        Ok(v.try_into().unwrap_or_else(|_| unreachable!()))
    }
    fn schema(s: &mut Schema) {
        s.wrap(&format!("[{N}]"), T::schema)
    }
}

impl<K: Wire + Ord, V: Wire> Wire for BTreeMap<K, V> {
    fn encode(&self, w: &mut Writer) {
        w.varint(self.len() as u64);
        for (k, v) in self {
            k.encode(w);
            v.encode(w);
        }
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let n = r.len(min_bytes::<K>())?;
        let mut out = BTreeMap::new();
        for _ in 0..n {
            let k = K::decode(r)?;
            out.insert(k, V::decode(r)?);
        }
        Ok(out)
    }
    fn schema(s: &mut Schema) {
        s.wrap("map", |s| {
            K::schema(s);
            s.sep();
            V::schema(s);
        })
    }
}

impl<K: Wire + Ord> Wire for BTreeSet<K> {
    fn encode(&self, w: &mut Writer) {
        w.varint(self.len() as u64);
        for k in self {
            k.encode(w);
        }
    }
    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let n = r.len(min_bytes::<K>())?;
        let mut out = BTreeSet::new();
        for _ in 0..n {
            out.insert(K::decode(r)?);
        }
        Ok(out)
    }
    fn schema(s: &mut Schema) {
        s.wrap("set", K::schema)
    }
}

macro_rules! tuple {
    ($($n:ident),+) => {
        impl<$($n: Wire),+> Wire for ($($n,)+) {
            #[allow(non_snake_case)]
            fn encode(&self, w: &mut Writer) { let ($($n,)+) = self; $($n.encode(w);)+ }
            fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> { Ok(($($n::decode(r)?,)+)) }
            fn schema(s: &mut Schema) { s.wrap("tuple", |s| { $($n::schema(s); s.sep();)+ }) }
        }
    };
}
tuple!(A);
tuple!(A, B);
tuple!(A, B, C);
tuple!(A, B, C, D);

#[cfg(feature = "glam")]
mod glam_impls {
    use super::*;
    macro_rules! glam_ty {
        ($t:ty, $name:literal, [$($f:ident),+]) => {
            impl Wire for $t {
                #[inline] fn encode(&self, w: &mut Writer) { $(self.$f.encode(w);)+ }
                #[inline] fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
                    $(let $f = f32::decode(r)?;)+
                    Ok(<$t>::from_array([$($f),+]))
                }
                fn schema(s: &mut Schema) { s.leaf($name) }
            }
        };
    }
    glam_ty!(glam::Vec2, "vec2", [x, y]);
    glam_ty!(glam::Vec3, "vec3", [x, y, z]);
    glam_ty!(glam::Vec4, "vec4", [x, y, z, w]);
    glam_ty!(glam::Quat, "quat", [x, y, z, w]);
}

#[cfg(test)]
mod tests;
