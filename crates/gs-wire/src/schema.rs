//! Canonical schema text + fingerprint.
//!
//! `describe::<T>()` renders a stable, human-readable description of `T`'s wire layout, e.g.
//! `Move{dx:f32,dy:f32,jump:bool}`. Named types are expanded once and referenced as `@Name`
//! afterwards, so recursive types terminate. `fingerprint::<T>()` is FNV-1a-64 of that text.

use std::collections::BTreeSet;

#[derive(Default)]
pub struct Schema {
    out: String,
    seen: BTreeSet<String>,
}

impl Schema {
    pub fn leaf(&mut self, name: &str) {
        self.out.push_str(name);
    }
    pub fn wrap(&mut self, name: &str, inner: impl FnOnce(&mut Schema)) {
        self.out.push_str(name);
        self.out.push('<');
        inner(self);
        if self.out.ends_with(',') {
            self.out.pop();
        }
        self.out.push('>');
    }
    /// Separator between the parameters of a multi-parameter wrapper (map, tuple).
    pub fn sep(&mut self) {
        self.out.push(',');
    }
    pub fn named(&mut self, name: String, body: impl FnOnce(&mut Schema)) {
        if !self.seen.insert(name.clone()) {
            self.out.push('@');
            self.out.push_str(&name);
            return;
        }
        self.out.push_str(&name);
        body(self);
    }
    pub fn record(&mut self, fields: impl FnOnce(&mut Schema)) {
        self.out.push('{');
        fields(self);
        if self.out.ends_with(',') {
            self.out.pop();
        }
        self.out.push('}');
    }
    pub fn choice(&mut self, variants: impl FnOnce(&mut Schema)) {
        self.out.push('(');
        variants(self);
        if self.out.ends_with('|') {
            self.out.pop();
        }
        self.out.push(')');
    }
    pub fn field(&mut self, name: &str, ty: impl FnOnce(&mut Schema)) {
        self.out.push_str(name);
        self.out.push(':');
        ty(self);
        self.out.push(',');
    }
    pub fn variant(&mut self, name: &str, fields: impl FnOnce(&mut Schema)) {
        self.out.push_str(name);
        let mark = self.out.len();
        self.record(fields);
        // A unit variant renders as just its name.
        if &self.out[mark..] == "{}" {
            self.out.truncate(mark);
        }
        self.out.push('|');
    }
}

/// Canonical schema text for `T`.
pub fn describe<T: crate::Wire>() -> String {
    let mut s = Schema::default();
    T::schema(&mut s);
    s.out
}

/// 64-bit fingerprint of `T`'s schema (FNV-1a over [`describe`]).
pub fn fingerprint<T: crate::Wire>() -> u64 {
    fnv1a64(describe::<T>().as_bytes())
}

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// `std::any::type_name` with module paths stripped: `a::b::Msg<c::Input>` → `Msg<Input>`.
pub fn short_type_name<T: ?Sized>() -> String {
    let full = std::any::type_name::<T>();
    let mut out = String::with_capacity(full.len());
    let mut seg = String::new();
    for ch in full.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            seg.push(ch);
        } else if ch == ':' {
            seg.clear();
        } else {
            out.push_str(&seg);
            seg.clear();
            out.push(ch);
        }
    }
    out.push_str(&seg);
    out.replace(' ', "")
}
