#![doc = include_str!("../README.md")]

use compact_str::{CompactString, format_compact};
use derive_more::{Constructor, Deref, DerefMut};
use pastey::paste;
use serde::de::{self, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;

#[derive(Debug)]
pub enum Path<'p> {
    Root,
    Index(&'p Path<'p>, usize),
    Key(&'p Path<'p>, CompactString),
}

impl fmt::Display for Path<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root => formatter.write_str("<root>"),
            Self::Index(Self::Root, index) => write!(formatter, "[{index}]"),
            Self::Index(parent, index) => write!(formatter, "{parent}[{index}]"),
            Self::Key(Self::Root, key) => formatter.write_str(key),
            Self::Key(parent, key) => write!(formatter, "{parent}.{key}"),
        }
    }
}

pub fn deserialize<'de, D, T, F>(deserializer: D, on_ignored: F) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
    F: FnMut(&Path<'_>),
{
    let mut state = State {
        on_ignored,
        key: None,
        index: 0,
    };
    let path = Path::Root;
    T::deserialize(Tracker::new(deserializer, &mut state, &path))
}

struct State<F> {
    on_ignored: F,
    key: Option<CompactString>,
    index: usize,
}

impl<F: FnMut(&Path<'_>)> State<F> {
    fn scope<T>(&mut self, visit: impl FnOnce(&mut Self) -> T) -> T {
        let context = (self.key.take(), self.index);
        self.index = 0;
        let result = visit(self);
        (self.key, self.index) = context;
        result
    }
}

#[derive(Constructor, Deref, DerefMut)]
struct Tracker<'s, 'p, T, F> {
    inner: T,
    #[deref(forward)]
    #[deref_mut(forward)]
    state: &'s mut State<F>,
    path: &'p Path<'p>,
}

#[derive(Constructor)]
struct Struct<T> {
    inner: T,
    fields: &'static [&'static str],
}

#[derive(Constructor)]
struct Field<'f, S> {
    inner: &'f mut Option<S>,
    fields: &'static [&'static str],
}

macro_rules! delegate {
    (@de $($method:ident $(($($argument:ident: $ty:ty),*))?),* $(,)?) => {
        paste! {$(
            fn [<deserialize_ $method>]<V: Visitor<'de>>(self, $($($argument: $ty,)*)? visitor: V) -> Result<V::Value, Self::Error> {
                self.inner.[<deserialize_ $method>]($($($argument,)*)? Tracker::new(visitor, self.state, self.path))
            }
        )*}
    };
    (@visit $($method:ident $(($($ty:ty)?))?),* $(,)?) => {
        $( delegate!(#impl visit $method $(($($ty)?))?); )*
    };
    (@key $($method:ident $(($ty:ty))?),* $(,)?) => {
        $( delegate!(#impl key $method $(($ty))?); )*
    };
    (#impl $mode:ident $method:ident) => {
        delegate!(#impl $mode $method ($method));
    };
    (#impl visit $method:ident ()) => {
        paste! {
            fn [<visit_ $method>]<E: de::Error>(self) -> Result<Self::Value, E> {
                self.inner.[<visit_ $method>]()
            }
        }
    };
    (#impl visit $method:ident ($ty:ty)) => {
        paste! {
            fn [<visit_ $method>]<E: de::Error>(self, value: $ty) -> Result<Self::Value, E> {
                self.inner.[<visit_ $method>](value)
            }
        }
    };
    (#impl key $method:ident ($ty:ty)) => {
        paste! {
            fn [<visit_ $method>]<E: de::Error>(mut self, value: $ty) -> Result<Self::Value, E> {
                if self.key.is_some() {
                    self.key = Some(format_compact!("{value}"));
                }
                self.inner.[<visit_ $method>](value)
            }
        }
    };
    (@field @$mode:ident $($method:ident ($ty:ty, $deserializer:ident)),* $(,)?) => {
        paste! {$(
            fn [<visit_ $method>]<E: de::Error>(self, value: $ty) -> Result<Self::Value, E> {
                delegate!(#impl field @$mode self(value, $deserializer))
            }
        )*}
    };
    (#impl field @key $receiver:ident ($value:ident, $deserializer:ident)) => {
        if $receiver.fields.contains(&$value.as_ref()) {
            delegate!(#impl field @plain $receiver($value, $deserializer))
        } else {
            Ok(None)
        }
    };
    (#impl field @plain $receiver:ident ($value:ident, $deserializer:ident)) => {
        $receiver.inner.take().unwrap()
            .deserialize(de::value::$deserializer::new($value))
            .map(Some)
    };
    (@plain $(fn $method:ident (&$receiver:ident $(, $argument:ident: $ty:ty)*) -> $result:ty;)+) => {
        $( delegate!(#impl plain [&] fn $method($receiver $(, $argument: $ty)*) -> $result;); )+
    };
    (@plain $(fn $method:ident ($receiver:ident $(, $argument:ident: $ty:ty)*) -> $result:ty;)+) => {
        $( delegate!(#impl plain [] fn $method($receiver $(, $argument: $ty)*) -> $result;); )+
    };
    (#impl plain [$($reference:tt)?] fn $method:ident ($receiver:ident $(, $argument:ident: $ty:ty)*) -> $result:ty;) => {
        fn $method($($reference)? $receiver $(, $argument: $ty)*) -> $result {
            $receiver.inner.$method($($argument),*)
        }
    };
    (@$mode:ident $(
        fn $method:ident <$generic:ident: $bound:ident<'de>> (
            $receiver:ident,
            $wrapped:ident: $ty:ty
        ) -> $result:ty;
    )+) => {
        $(fn $method<$generic: $bound<'de>>($receiver, $wrapped: $ty) -> $result {
            delegate!(#impl $mode $receiver.$method($wrapped))
        })+
    };
    (@$mode:ident $(
        fn $method:ident <$generic:ident: $bound:ident<'de>> (
            $receiver:ident,
            $argument:ident: $ty:ty,
            $wrapped:ident: $input:ty
        ) -> $result:ty;
    )+) => {
        $(fn $method<$generic: $bound<'de>>($receiver, $argument: $ty, $wrapped: $input) -> $result {
            delegate!(#impl $mode $receiver.$method($wrapped, $argument))
        })+
    };
    (#impl wrap $receiver:ident.$method:ident ($wrapped:ident $(, $argument:ident)?)) => {
        $receiver.inner.$method($($argument,)? Tracker::new($wrapped, $receiver.state, $receiver.path))
    };
    (#impl scope $receiver:ident.$method:ident ($wrapped:ident $(, $argument:ident)?)) => {
        $receiver.state.scope(|state| $receiver.inner.$method($($argument,)? Tracker::new($wrapped, state, $receiver.path)))
    };
    (#impl plain $receiver:ident.$method:ident ($wrapped:ident $(, $argument:ident)?)) => {
        $receiver.inner.$method($($argument,)? $wrapped)
    };
}

impl<'de, 's, 'p, D: Deserializer<'de>, F: FnMut(&Path<'_>)> Deserializer<'de>
    for Tracker<'s, 'p, D, F>
{
    type Error = D::Error;

    delegate! { @de
        any, bool, char, i8, i16, i32, i64, i128, u8, u16, u32, u64, u128, f32, f64,
        str, string, bytes, byte_buf, option, unit, seq, map, identifier,
        unit_struct(name: &'static str), newtype_struct(name: &'static str), tuple(len: usize),
        tuple_struct(name: &'static str, len: usize),
        enum(name: &'static str, variants: &'static [&'static str])
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_struct(
            name,
            fields,
            Tracker::new(Struct::new(visitor, fields), self.state, self.path),
        )
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        (self.state.on_ignored)(self.path);
        self.inner.deserialize_ignored_any(visitor)
    }

    delegate! { @plain
        fn is_human_readable(&self) -> bool;
    }
}

impl<'de, 's, 'p, V: Visitor<'de>, F: FnMut(&Path<'_>)> Visitor<'de> for Tracker<'s, 'p, V, F> {
    type Value = V::Value;
    delegate! { @plain
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result;
    }

    delegate! { @key
        bool, i8, i16, i32, i64, i128, u8, u16, u32, u64, u128, f32, f64, char,
        str(&str), borrowed_str(&'de str), string(String),
    }
    delegate! { @visit
        none(), unit(), bytes(&[u8]), borrowed_bytes(&'de [u8]), byte_buf(Vec<u8>),
    }

    delegate! { @wrap
        fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error>;
        fn visit_newtype_struct<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error>;
    }
    delegate! { @scope
        fn visit_seq<A: SeqAccess<'de>>(self, sequence: A) -> Result<Self::Value, A::Error>;
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error>;
        fn visit_enum<A: EnumAccess<'de>>(self, enumeration: A) -> Result<Self::Value, A::Error>;
    }
}

impl<'de, V: Visitor<'de>> Visitor<'de> for Struct<V> {
    type Value = V::Value;

    delegate! { @plain
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result;
    }
    delegate! { @visit
        bool, i8, i16, i32, i64, i128, u8, u16, u32, u64, u128, f32, f64, char,
        str(&str), borrowed_str(&'de str), string(String),
        none(), unit(), bytes(&[u8]), borrowed_bytes(&'de [u8]), byte_buf(Vec<u8>),
    }
    delegate! { @plain
        fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error>;
        fn visit_newtype_struct<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error>;
        fn visit_seq<A: SeqAccess<'de>>(self, sequence: A) -> Result<Self::Value, A::Error>;
        fn visit_enum<A: EnumAccess<'de>>(self, enumeration: A) -> Result<Self::Value, A::Error>;
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        self.inner.visit_map(Struct::new(map, self.fields))
    }
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for Struct<A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        let mut seed = Some(seed);
        loop {
            match self
                .inner
                .next_key_seed(Field::new(&mut seed, self.fields))?
            {
                Some(None) => {
                    self.inner.next_value::<de::IgnoredAny>()?;
                }
                key => return Ok(key.flatten()),
            }
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        self.inner.next_value_seed(seed)
    }
}

impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for Field<'_, S> {
    type Value = Option<S::Value>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_identifier(self)
    }
}

impl<'de, S: DeserializeSeed<'de>> Visitor<'de> for Field<'_, S> {
    type Value = Option<S::Value>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a struct field")
    }

    delegate! { @field @key
        str(&str, StrDeserializer),
        borrowed_str(&'de str, BorrowedStrDeserializer),
        string(String, StringDeserializer),
    }
    delegate! { @field @plain
        u64(u64, U64Deserializer),
        bytes(&[u8], BytesDeserializer),
        borrowed_bytes(&'de [u8], BorrowedBytesDeserializer),
    }
}

impl<'de, 's, 'p, S: DeserializeSeed<'de>, F: FnMut(&Path<'_>)> DeserializeSeed<'de>
    for Tracker<'s, 'p, S, F>
{
    type Value = S::Value;
    delegate! { @wrap
        fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error>;
    }
}

impl<'de, 's, 'p, A: SeqAccess<'de>, F: FnMut(&Path<'_>)> SeqAccess<'de> for Tracker<'s, 'p, A, F> {
    type Error = A::Error;
    fn next_element_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, A::Error> {
        let path = Path::Index(self.path, self.index);
        let result = self
            .inner
            .next_element_seed(Tracker::new(seed, self.state, &path));
        if matches!(result, Ok(Some(_))) {
            self.index += 1;
        }
        result
    }
    delegate! { @plain
        fn size_hint(&self) -> Option<usize>;
    }
}

impl<'de, 's, 'p, A: MapAccess<'de>, F: FnMut(&Path<'_>)> MapAccess<'de> for Tracker<'s, 'p, A, F> {
    type Error = A::Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        self.key = Some(CompactString::const_new("?"));
        self.inner
            .next_key_seed(Tracker::new(seed, self.state, self.path))
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        let key = self
            .key
            .take()
            .unwrap_or_else(|| CompactString::const_new("?"));
        let path = Path::Key(self.path, key);
        self.inner
            .next_value_seed(Tracker::new(seed, self.state, &path))
    }
    delegate! { @plain
        fn size_hint(&self) -> Option<usize>;
    }
}

impl<'de, 's, 'p, A: EnumAccess<'de>, F: FnMut(&Path<'_>)> EnumAccess<'de>
    for Tracker<'s, 'p, A, F>
{
    type Error = A::Error;
    type Variant = Tracker<'s, 'p, A::Variant, F>;
    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), A::Error> {
        let (value, access) = self.inner.variant_seed(seed)?;
        Ok((value, Tracker::new(access, self.state, self.path)))
    }
}

impl<'de, 's, 'p, A: VariantAccess<'de>, F: FnMut(&Path<'_>)> VariantAccess<'de>
    for Tracker<'s, 'p, A, F>
{
    type Error = A::Error;
    delegate! { @plain
        fn unit_variant(self) -> Result<(), Self::Error>;
    }
    delegate! { @wrap
        fn newtype_variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<S::Value, Self::Error>;
    }
    delegate! { @wrap
        fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>;
        fn struct_variant<V: Visitor<'de>>(self, fields: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::borrow::Cow;
    use std::collections::{BTreeMap, HashMap};

    #[test]
    fn paths_display_root_keys_and_indices() {
        let root = Path::Root;
        let key = Path::Key(&root, CompactString::const_new("items"));
        let index = Path::Index(&key, 2);
        let nested_key = Path::Key(&index, CompactString::const_new("extra"));
        let root_index = Path::Index(&root, 0);
        let nested_index = Path::Index(&root_index, 3);
        let indexed_key = Path::Key(&root_index, CompactString::const_new("extra"));
        for (path, expected) in [
            (&root, "<root>"),
            (&key, "items"),
            (&index, "items[2]"),
            (&nested_key, "items[2].extra"),
            (&root_index, "[0]"),
            (&nested_index, "[0][3]"),
            (&indexed_key, "[0].extra"),
        ] {
            assert_eq!(path.to_string(), expected);
        }
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Inner {
        name: String,
        #[serde(default)]
        number: Option<u32>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Meters(f64);

    #[derive(Debug, Deserialize, PartialEq)]
    enum Kind {
        A,
        B(Inner),
        C { x: Inner },
        D(u8, Inner),
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Outer {
        title: String,
        inner: Option<Inner>,
        items: Vec<Inner>,
        by_name: HashMap<String, Inner>,
        by_id: BTreeMap<u32, Inner>,
        len: Meters,
        kind: Kind,
        kinds: Vec<Kind>,
    }

    fn run<T>(json: &str) -> (T, Vec<String>)
    where
        T: for<'de> Deserialize<'de> + PartialEq + fmt::Debug,
    {
        let mut ignored = Vec::new();
        let mut deserializer = serde_json::Deserializer::from_str(json);
        let value: T =
            deserialize(&mut deserializer, |path| ignored.push(path.to_string())).unwrap();
        deserializer.end().unwrap();
        let plain: T = serde_json::from_str(json).unwrap();
        assert_eq!(value, plain, "wrapper must not change the result");
        (value, ignored)
    }

    #[test]
    fn reports_nested_unknown_fields_with_paths() {
        let json = r#"{
            "title": "t",
            "extra": {"deep": [1, 2, {"x": 1}]},
            "inner": {"name": "a", "zzz": 1},
            "items": [{"name": "i0"}, {"name": "i1", "nope": null}],
            "by_name": {"foo": {"name": "f", "bad": true}},
            "by_id": {"7": {"name": "seven", "huh": 1}},
            "len": 1.5,
            "kind": {"B": {"name": "b", "oops": 1}},
            "kinds": ["A", {"C": {"x": {"name": "c", "k": 0}}}, {"D": [1, {"name": "d", "q": 2}]}],
            "ex\u0074ra2": "escaped key"
        }"#;
        let (_, ignored): (Outer, _) = run(json);
        assert_eq!(
            ignored,
            [
                "extra",
                "inner.zzz",
                "items[1].nope",
                "by_name.foo.bad",
                "by_id.7.huh",
                "kind.oops",
                "kinds[1].x.k",
                "kinds[2][1].q",
                "extra2",
            ]
        );
    }

    #[test]
    fn strict_structs_filter_unknown_fields_without_filtering_map_keys() {
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct StrictInner {
            name: String,
        }

        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Strict {
            #[serde(alias = "title")]
            name: String,
            by_name: BTreeMap<String, StrictInner>,
            items: Vec<StrictInner>,
        }

        let json = r#"{
            "before": 1,
            "title": "root",
            "middle": {"nested": [1, 2]},
            "by_name": {"arbitrary": {"unknown": 1, "name": "map"}},
            "items": [{"unknown": 1, "name": "item"}],
            "after": null
        }"#;
        assert!(serde_json::from_str::<Strict>(json).is_err());

        let mut ignored = Vec::new();
        let mut deserializer = serde_json::Deserializer::from_str(json);
        let value: Strict =
            deserialize(&mut deserializer, |path| ignored.push(path.to_string())).unwrap();
        deserializer.end().unwrap();

        assert_eq!(value.name, "root");
        assert_eq!(value.by_name["arbitrary"].name, "map");
        assert_eq!(value.items[0].name, "item");
        assert_eq!(
            ignored,
            [
                "before",
                "middle",
                "by_name.arbitrary.unknown",
                "items[0].unknown",
                "after",
            ]
        );
    }

    #[test]
    fn empty_structs_ignore_all_fields() {
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Empty {}

        let json = r#"{"first": 1, "second": {"nested": true}}"#;
        assert!(serde_json::from_str::<Empty>(json).is_err());

        let mut ignored = Vec::new();
        let mut deserializer = serde_json::Deserializer::from_str(json);
        let _: Empty =
            deserialize(&mut deserializer, |path| ignored.push(path.to_string())).unwrap();
        deserializer.end().unwrap();
        assert_eq!(ignored, ["first", "second"]);
    }

    #[test]
    fn structs_still_accept_sequences() {
        let (value, ignored): (Inner, _) = run(r#"["name", null]"#);
        assert_eq!(value.name, "name");
        assert!(ignored.is_empty());
    }

    #[test]
    fn struct_field_indices_and_bytes_are_forwarded() {
        fn check<'de, D: Deserializer<'de>>(deserializer: D) {
            let mut ignored = Vec::new();
            let value: Inner =
                deserialize(deserializer, |path| ignored.push(path.to_string())).unwrap();
            assert_eq!(value.name, "name");
            assert!(ignored.is_empty());
        }

        use de::value::{BorrowedBytesDeserializer, BytesDeserializer, Error, MapDeserializer};
        check(MapDeserializer::<_, Error>::new(
            [(0u64, "name")].into_iter(),
        ));
        check(MapDeserializer::<_, Error>::new(
            [(BytesDeserializer::new(b"name"), "name")].into_iter(),
        ));
        check(MapDeserializer::<_, Error>::new(
            [(BorrowedBytesDeserializer::new(b"name"), "name")].into_iter(),
        ));
    }

    #[test]
    fn non_string_map_keys_are_recorded_in_paths() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Keys {
            signed: BTreeMap<i64, Inner>,
            unsigned: BTreeMap<u8, Inner>,
            boolean: BTreeMap<bool, Inner>,
            character: BTreeMap<char, Inner>,
        }
        let json = r#"{
            "signed": {"-3":   {"name": "x", "z1": 0}},
            "unsigned": {"200":  {"name": "x", "z2": 0}},
            "boolean": {"true": {"name": "x", "z3": 0}},
            "character": {"x":    {"name": "x", "z4": 0}}
        }"#;
        let (_, ignored): (Keys, _) = run(json);
        assert_eq!(
            ignored,
            [
                "signed.-3.z1",
                "unsigned.200.z2",
                "boolean.true.z3",
                "character.x.z4"
            ]
        );
    }

    #[test]
    fn optional_newtype_map_keys_are_recorded_in_paths() {
        #[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
        struct Key(String);

        let json = r#"{"first":{"name":"a","unknown":1},"second":{"name":"b","extra":2}}"#;
        let (_, ignored): (BTreeMap<Option<Key>, Inner>, _) = run(json);
        assert_eq!(ignored, ["first.unknown", "second.extra"]);
    }

    #[test]
    fn floating_point_keys_keep_display_formatting() {
        #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
        struct Key(String);

        impl<'de> Deserialize<'de> for Key {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                f64::deserialize(deserializer).map(|value| Key(value.to_string()))
            }
        }

        let json = r#"{
            "1.0":{"name":"a","unknown":1},
            "-0.0":{"name":"b","unknown":2},
            "1e-8":{"name":"c","unknown":3},
            "1e20":{"name":"d","unknown":4}
        }"#;
        let (_, ignored): (BTreeMap<Key, Inner>, _) = run(json);
        assert_eq!(
            ignored,
            [
                "1.unknown",
                "-0.unknown",
                "0.00000001.unknown",
                "100000000000000000000.unknown",
            ]
        );
    }

    #[test]
    fn compound_map_keys_do_not_capture_their_contents() {
        use serde::de::value::MapDeserializer;

        #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
        struct Key(String);

        impl<'de> Deserialize<'de> for Key {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                serde_json::Value::deserialize(deserializer).map(|value| Key(value.to_string()))
            }
        }

        let entries = [
            (
                serde_json::json!([1, 2]),
                serde_json::json!({"name": "a", "sequence_unknown": 1}),
            ),
            (
                serde_json::json!({"nested": 3}),
                serde_json::json!({"name": "b", "map_unknown": 2}),
            ),
            (
                serde_json::json!("plain"),
                serde_json::json!({"name": "c", "scalar_unknown": 3}),
            ),
        ];
        let mut ignored = Vec::new();
        let deserializer = MapDeserializer::<_, serde_json::Error>::new(entries.into_iter());
        let values: BTreeMap<Key, Inner> =
            deserialize(deserializer, |path| ignored.push(path.to_string())).unwrap();
        assert_eq!(values.len(), 3);
        assert_eq!(
            ignored,
            [
                "?.sequence_unknown",
                "?.map_unknown",
                "plain.scalar_unknown"
            ]
        );
    }

    #[test]
    fn no_unknown_fields_means_no_callbacks() {
        let json = r#"{"title":"t","inner":null,"items":[],"by_name":{},"by_id":{},
                       "len":0,"kind":"A","kinds":[]}"#;
        let (value, ignored): (Outer, _) = run(json);
        assert!(ignored.is_empty());
        assert_eq!(value.inner, None);
    }

    #[test]
    fn nested_sequences_keep_independent_indices() {
        let json = r#"[
            [{"name":"a","x":1},{"name":"b","y":2}],
            [{"name":"c","z":3}]
        ]"#;
        let (_, ignored): (Vec<Vec<Inner>>, _) = run(json);
        assert_eq!(ignored, ["[0][0].x", "[0][1].y", "[1][0].z"]);
    }

    #[test]
    fn ignored_value_subtrees_report_once() {
        let value = serde_json::json!({"name":"a","extra":{"deep":[1,{"x":2}]}});
        let mut ignored = Vec::new();
        let inner: Inner = deserialize(value, |path| ignored.push(path.to_string())).unwrap();
        assert_eq!(inner.name, "a");
        assert_eq!(ignored, ["extra"]);
    }

    #[test]
    fn borrowed_strings_stay_zero_copy() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct BorrowedStrings<'i> {
            #[serde(borrow)]
            text: &'i str,
            #[serde(borrow)]
            cow_text: Cow<'i, str>,
        }
        let json = r#"{"text": "hello", "cow_text": "world", "unknown": 1}"#;
        let mut ignored = Vec::new();
        let mut deserializer = serde_json::Deserializer::from_str(json);
        let borrowed: BorrowedStrings =
            deserialize(&mut deserializer, |path| ignored.push(path.to_string())).unwrap();
        assert_eq!(borrowed.text, "hello");
        assert!(matches!(borrowed.cow_text, Cow::Borrowed("world")));
        assert_eq!(ignored, ["unknown"]);
    }

    #[test]
    fn errors_still_propagate() {
        let mut deserializer = serde_json::Deserializer::from_str(r#"{"title": 1}"#);
        let result: Result<Outer, _> = deserialize(&mut deserializer, |_| {});
        assert!(result.is_err());
    }
}
