use super::*;
use std::collections::BTreeMap;

#[derive(Wire, Debug, Clone, PartialEq, Default)]
struct Inner {
    a: u32,
    b: i16,
    name: String,
}

#[derive(Wire, Debug, Clone, PartialEq)]
enum Msg<T> {
    Ping,
    Data(T, Vec<u8>),
    Named {
        x: f32,
        inner: Option<Box<Inner>>,
        #[wire(skip)]
        cache: u64,
    },
}

#[derive(Wire, Debug, Clone, PartialEq)]
struct Tuple(u64, bool, [i8; 3]);

fn rt<T: Wire + PartialEq + std::fmt::Debug>(v: T) {
    let b = to_bytes(&v);
    assert_eq!(from_bytes::<T>(&b).unwrap(), v);
}

#[test]
fn roundtrips() {
    rt(0u64);
    rt(u64::MAX);
    rt(i64::MIN);
    rt(-1i32);
    // NaN payload preserved bit-exactly (NaN != NaN, so compare bits).
    assert_eq!(from_bytes::<f32>(&to_bytes(&f32::from_bits(0x7fc0_0001))).unwrap().to_bits(), 0x7fc0_0001);
    rt(String::from("héllo"));
    rt(Tuple(300, true, [-1, 0, 1]));
    rt(Msg::<u16>::Ping);
    rt(Msg::Data(7u16, vec![1, 2, 3]));
    rt(Msg::<u16>::Named { x: 1.5, inner: Some(Box::new(Inner { a: 9, b: -3, name: "x".into() })), cache: 0 });
    let mut m = BTreeMap::new();
    m.insert(3u8, Inner::default());
    rt(m);
    rt(glam::Vec3::new(1.0, -2.0, 3.5));
    rt((1u8, -2i64, Blob(vec![9; 40])));
}

#[test]
fn skip_fields_are_not_sent() {
    let a = Msg::<u8>::Named { x: 0.0, inner: None, cache: 99 };
    let b = from_bytes::<Msg<u8>>(&to_bytes(&a)).unwrap();
    assert_eq!(b, Msg::Named { x: 0.0, inner: None, cache: 0 });
}

#[test]
fn varint_sizes() {
    assert_eq!(to_bytes(&127u32).len(), 1);
    assert_eq!(to_bytes(&128u32).len(), 2);
    assert_eq!(to_bytes(&-1i32).len(), 1);
}

#[test]
fn hostile_inputs_error_not_panic() {
    // Forged huge length.
    let mut w = Writer::new();
    w.varint(u32::MAX as u64);
    assert!(matches!(from_bytes::<Vec<u32>>(w.as_slice()), Err(WireError::TooLong(_))));
    // Bad enum tag.
    assert_eq!(from_bytes::<Msg<u8>>(&[9]), Err(WireError::BadTag(9)));
    // Truncated.
    assert_eq!(from_bytes::<f32>(&[1, 2]), Err(WireError::Eof));
    // Trailing.
    assert_eq!(from_bytes::<u8>(&[1, 2]), Err(WireError::Trailing(1)));
    // Overlong varint.
    assert_eq!(from_bytes::<u64>(&[0xff; 11]), Err(WireError::VarintOverflow));
    // u16 overflow.
    assert_eq!(from_bytes::<u16>(&to_bytes(&70_000u32)), Err(WireError::VarintOverflow));
    // Bad utf8.
    assert_eq!(from_bytes::<String>(&[2, 0xff, 0xfe]), Err(WireError::Utf8));
    // Deep nesting.
    let deep = vec![1u8; 200];
    assert!(from_bytes::<Option<Box<Option<Box<Option<Box<u8>>>>>>>(&deep).is_err());
}

#[test]
fn fuzz_random_bytes_never_panic() {
    let mut seed = 0x1234_5678_u64;
    for _ in 0..20_000 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let n = (seed % 48) as usize;
        let bytes: Vec<u8> = (0..n).map(|i| (seed >> (i % 8 * 8)) as u8 ^ i as u8).collect();
        let _ = from_bytes::<Msg<Inner>>(&bytes);
        let _ = from_bytes::<Vec<Msg<String>>>(&bytes);
        let _ = from_bytes::<BTreeMap<u16, Tuple>>(&bytes);
    }
}

#[test]
fn schema_is_canonical_and_sensitive() {
    assert_eq!(describe::<Inner>(), "Inner{a:u32,b:i16,name:string}");
    assert_eq!(
        describe::<Msg<u16>>(),
        "Msg<u16>(Ping|Data{0:u16,1:vec<u8>}|Named{x:f32,inner:option<Inner{a:u32,b:i16,name:string}>})"
    );
    assert_ne!(fingerprint::<Msg<u16>>(), fingerprint::<Msg<u32>>());
    assert_eq!(short_type_name::<Option<std::string::String>>(), "Option<String>");
}
