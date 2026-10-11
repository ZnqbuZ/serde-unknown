#![cfg_attr(not(feature = "std"), no_std)]
#![doc = include_str!("../README.md")]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use compact_str::{CompactString, format_compact};
use core::fmt;
use derive_more::{Constructor, Deref, DerefMut};
use pastey::paste;
use serde::de::{self, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde::{Deserialize, Deserializer};

#[derive(Debug)]
pub enum Path<'p> {
    Root,
    Index(&'p Path<'p>, usize),
    Key(&'p Path<'p>, CompactString),
}

impl<'p> Path<'p> {
    pub fn matches(&self, pattern: &[&str]) -> bool {
        let mut path = self;
        let mut segments = pattern.iter().rev();
        while let Some(&segment) = segments.next() {
            path = match path {
                Self::Root => return segment == "<root>" && segments.next().is_none(),
                Self::Index(parent, index) => {
                    if segment != "*" && segment.parse::<usize>().ok() != Some(*index) {
                        return false;
                    }
                    parent
                }
                Self::Key(parent, key) => {
                    if segment != "*" && segment != key.as_str() {
                        return false;
                    }
                    parent
                }
            };
        }
        false
    }
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

pub fn deserialize<'de, D, T, F>(deserializer: D, on_unknown: F) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
    F: FnMut(&Path<'_>) -> Result<(), D::Error>,
{
    let mut state = State {
        on_unknown,
        error: None,
        key: None,
        index: 0,
    };
    let path = Path::Root;
    let result = T::deserialize(Tracker::new(deserializer, &mut state, &path));
    match state.error {
        Some(error) => Err(error),
        None => result,
    }
}

pub trait DeserializerExt<'de>: Deserializer<'de> {
    fn deserialize<T, F>(self, on_unknown: F) -> Result<T, Self::Error>
    where
        T: Deserialize<'de>,
        F: FnMut(&Path<'_>) -> Result<(), Self::Error>,
    {
        deserialize(self, on_unknown)
    }
}

impl<'de, D: Deserializer<'de>> DeserializerExt<'de> for D {}

struct State<E, F> {
    on_unknown: F,
    error: Option<E>,
    key: Option<CompactString>,
    index: usize,
}

impl<E, F: FnMut(&Path<'_>) -> Result<(), E>> State<E, F> {
    fn unknown<Error: de::Error>(&mut self, path: &Path<'_>) -> Result<(), Error> {
        if self.error.is_none() {
            match (self.on_unknown)(path) {
                Ok(()) => return Ok(()),
                Err(error) => self.error = Some(error),
            }
        }
        Err(Error::custom("unknown field rejected"))
    }

    fn scope<T>(&mut self, visit: impl FnOnce(&mut Self) -> T) -> T {
        let context = (self.key.take(), self.index);
        self.index = 0;
        let result = visit(self);
        (self.key, self.index) = context;
        result
    }
}

#[derive(Constructor, Deref, DerefMut)]
struct Tracker<'s, 'p, T, E, F> {
    inner: T,
    #[deref(forward)]
    #[deref_mut(forward)]
    state: &'s mut State<E, F>,
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
            fn [<visit_ $method>]<Error: de::Error>(self) -> Result<Self::Value, Error> {
                self.inner.[<visit_ $method>]()
            }
        }
    };
    (#impl visit $method:ident ($ty:ty)) => {
        paste! {
            fn [<visit_ $method>]<Error: de::Error>(self, value: $ty) -> Result<Self::Value, Error> {
                self.inner.[<visit_ $method>](value)
            }
        }
    };
    (#impl key $method:ident ($ty:ty)) => {
        paste! {
            fn [<visit_ $method>]<Error: de::Error>(mut self, value: $ty) -> Result<Self::Value, Error> {
                if self.key.is_some() {
                    self.key = Some(format_compact!("{value}"));
                }
                self.inner.[<visit_ $method>](value)
            }
        }
    };
    (@field @$mode:ident $($method:ident ($ty:ty, $deserializer:ident)),* $(,)?) => {
        paste! {$(
            fn [<visit_ $method>]<Error: de::Error>(self, value: $ty) -> Result<Self::Value, Error> {
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

impl<'de, 's, 'p, D: Deserializer<'de>, E, F: FnMut(&Path<'_>) -> Result<(), E>> Deserializer<'de>
    for Tracker<'s, 'p, D, E, F>
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
        self.state.unknown(self.path)?;
        self.inner.deserialize_ignored_any(visitor)
    }

    delegate! { @plain
        fn is_human_readable(&self) -> bool;
    }
}

impl<'de, 's, 'p, V: Visitor<'de>, E, F: FnMut(&Path<'_>) -> Result<(), E>> Visitor<'de>
    for Tracker<'s, 'p, V, E, F>
{
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

impl<'de, 's, 'p, S: DeserializeSeed<'de>, E, F: FnMut(&Path<'_>) -> Result<(), E>>
    DeserializeSeed<'de> for Tracker<'s, 'p, S, E, F>
{
    type Value = S::Value;
    delegate! { @wrap
        fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error>;
    }
}

impl<'de, 's, 'p, A: SeqAccess<'de>, E, F: FnMut(&Path<'_>) -> Result<(), E>> SeqAccess<'de>
    for Tracker<'s, 'p, A, E, F>
{
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

impl<'de, 's, 'p, A: MapAccess<'de>, E, F: FnMut(&Path<'_>) -> Result<(), E>> MapAccess<'de>
    for Tracker<'s, 'p, A, E, F>
{
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

impl<'de, 's, 'p, A: EnumAccess<'de>, E, F: FnMut(&Path<'_>) -> Result<(), E>> EnumAccess<'de>
    for Tracker<'s, 'p, A, E, F>
{
    type Error = A::Error;
    type Variant = Tracker<'s, 'p, A::Variant, E, F>;
    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), A::Error> {
        let (value, access) = self.inner.variant_seed(seed)?;
        Ok((value, Tracker::new(access, self.state, self.path)))
    }
}

impl<'de, 's, 'p, A: VariantAccess<'de>, E, F: FnMut(&Path<'_>) -> Result<(), E>> VariantAccess<'de>
    for Tracker<'s, 'p, A, E, F>
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
