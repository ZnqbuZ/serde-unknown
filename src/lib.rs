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

/// A path represented by nodes that borrow their parents.
///
/// Callbacks receive a temporary path. [`Path::matches`] compares its segments
/// without allocating. [`Display`](fmt::Display) prints keys separated by dots
/// and indices in brackets, omitting the root prefix for non-root paths.
#[derive(Debug)]
pub enum Path<'p> {
    /// The root value, displayed as `<root>`.
    Root,
    /// A sequence element with its parent path and zero-based index.
    Index(&'p Path<'p>, usize),
    /// A map or struct value with its parent path and key.
    Key(&'p Path<'p>, CompactString),
}

impl<'p> Path<'p> {
    /// Matches the entire path against root-to-leaf segments without allocating.
    ///
    /// The first segment must be `"<root>"`. `"*"` matches exactly one key or
    /// sequence index, but cannot replace the root segment. Indices are compared
    /// as `usize` values. Dots and brackets inside keys are literal; `"*"` has no
    /// escape syntax. Matching takes linear time and constant auxiliary space.
    ///
    /// ```
    /// use serde_unknown::Path;
    ///
    /// let root = Path::Root;
    /// let items = Path::Key(&root, "items".into());
    /// let entry = Path::Index(&items, 3);
    /// let extra = Path::Key(&entry, "extra".into());
    ///
    /// assert!(extra.matches(&["<root>", "items", "3", "extra"]));
    /// assert!(extra.matches(&["<root>", "items", "*", "extra"]));
    /// assert!(!extra.matches(&["items", "3", "extra"]));
    /// ```
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

macro_rules! implement {
    ([$($generic:tt)*] ($($header:tt)*) -> $error:ty; $($body:tt)*) => {
        #[cfg(feature = "peek")]
        implement! { #impl [$($generic)*] ($($header)*) where [
            F: FnMut(&Path<'_>) -> Result<Option<C>, $error>,
            P: FnMut(&Path<'_>, Option<C>, serde_json::Value) -> Result<(), $error>,
        ];
            $($body)*
        }

        #[cfg(not(feature = "peek"))]
        implement! { #impl [$($generic)*] ($($header)*) where [
            F: FnMut(&Path<'_>) -> Result<Option<C>, $error>,
        ];
            $($body)*
        }
    };
    (@builder $missing:ident [$($generic:tt)*] ($($header:tt)*) -> $error:ty;
        $context:ty $(where [$($bound:tt)*])?
    ) => {
        implement! { #impl [$($generic)*] ($($header)*) $(where [$($bound)*])?;
            /// Deserializes `T` using the configured callbacks.
            ///
            /// Returns the first callback error unchanged, or the underlying
            /// deserializer's result when no callback rejects the input.
            pub fn deserialize<T: Deserialize<'de>>(self) -> Result<T, $error> {
                Builder {
                    deserializer: self.deserializer,
                    filter: implement!(#field $missing self.filter ($context, $error)),
                    peek: implement!(#field $missing self.peek ($context, $error)),
                }
                .deserialize()
            }
        }
    };
    (#impl [$($generic:tt)*] ($($header:tt)*) $(where [$($bound:tt)*])?; $($body:tt)*) => {
        impl<$($generic)*> $($header)* $(where $($bound)*)? {
            $($body)*
        }
    };
    (#field filter $receiver:ident.filter ($context:ty, $error:ty)) => {
        None::<fn(&Path<'_>) -> Result<Option<$context>, $error>>
    };
    (#field peek $receiver:ident.peek ($context:ty, $error:ty)) => {
        None::<fn(&Path<'_>, Option<$context>, serde_json::Value) -> Result<(), $error>>
    };
    (#field $missing:ident $receiver:ident.$field:ident ($context:ty, $error:ty)) => {
        $receiver.$field
    };
}

/// Unknown-field handling for any Serde [`Deserializer`].
///
/// Import this trait to call [`.deserialize(callback)`](Self::deserialize) or
/// [`.track()`](Self::track) on a deserializer. The adapter supports structs that
/// would otherwise reject unknown fields, including pbjson-generated messages.
/// Known values retain the target type's deserialization behavior.
///
/// Unknown-field tracking does not support `#[serde(flatten)]`. Paths borrow
/// stack-local nodes and cannot be kept after the callback returns.
pub trait DeserializerExt<'de>: Deserializer<'de> {
    /// Deserializes `T`, calling `callback` before each unknown value is consumed.
    ///
    /// Return `Ok(())` to continue or `Self::Error` to reject the input. The first
    /// callback error is returned unchanged even if nested deserialization uses
    /// a different error type or catches the intermediate error. No further
    /// callbacks run after rejection. An unknown subtree produces one callback.
    ///
    /// ```
    /// use serde::Deserialize;
    /// use serde_unknown::DeserializerExt;
    ///
    /// #[derive(Deserialize)]
    /// #[serde(deny_unknown_fields)]
    /// struct Config { port: u16 }
    ///
    /// let mut deserializer = serde_json::Deserializer::from_str(
    ///     r#"{"port":8080,"future":true}"#,
    /// );
    /// let mut count = 0;
    /// let config: Config = (&mut deserializer).deserialize(|path| {
    ///     assert!(path.matches(&["<root>", "future"]));
    ///     count += 1;
    ///     Ok(())
    /// })?;
    /// deserializer.end()?;
    /// assert_eq!(config.port, 8080);
    /// assert_eq!(count, 1);
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    fn deserialize<T, F>(self, mut callback: F) -> Result<T, Self::Error>
    where
        T: Deserialize<'de>,
        F: FnMut(&Path<'_>) -> Result<(), Self::Error>,
    {
        self.track()
            .filter(move |path| callback(path).map(|()| None::<()>))
            .deserialize()
    }

    /// Creates a [`Builder`] with neither callback configured.
    ///
    /// Use [`Builder::filter`] to select unknown values or reject their paths.
    /// With the `peek` feature, `Builder::peek` can inspect their parsed values.
    /// With neither callback, unknown values are consumed directly.
    fn track(self) -> Builder<Self> {
        Builder {
            deserializer: self,
            filter: None,
            peek: None,
        }
    }
}

impl<'de, D: Deserializer<'de>> DeserializerExt<'de> for D {}

/// Configures optional callbacks before deserializing a value.
///
/// Created by [`DeserializerExt::track`]. Callbacks retain their concrete types
/// and need no boxing. [`filter`](Self::filter) and `peek` (with the `peek`
/// feature) can be configured in either order. Call `.deserialize()` to execute.
///
/// | Callbacks | Behavior |
/// | --- | --- |
/// | Neither | Consume unknown values without calling user code. |
/// | Filter only | Run the filter and consume values directly. |
/// | Peek only | Inspect every unknown value with context `None::<()>`. |
/// | Both | Inspect selected values with the filter's `Some(context)`. |
///
/// Filter and peek errors reject the input. The first callback error is returned
/// unchanged, even if nested code catches the intermediate error.
#[must_use]
pub struct Builder<D, F = (), P = ()> {
    deserializer: D,
    filter: Option<F>,
    peek: Option<P>,
}

impl<D, F, P> Builder<D, F, P> {
    /// Sets a callback that runs before an unknown value is consumed.
    ///
    /// Return `Some(context)` to select the value for `peek`, `None` to consume
    /// it directly, or `D::Error` to reject the input. The context type `C` is
    /// chosen by the caller; `peek` receives `Some(context)` for selected values.
    /// Without a peek callback, both successful results consume values directly.
    /// Calling this method again replaces the previous filter.
    ///
    /// ```
    /// use serde::{Deserialize, de::Error};
    /// use serde_unknown::DeserializerExt;
    ///
    /// #[derive(Deserialize)]
    /// struct Config { port: u16 }
    ///
    /// let mut deserializer = serde_json::Deserializer::from_str(
    ///     r#"{"port":8080,"forbidden":true}"#,
    /// );
    /// let result: Result<Config, _> = (&mut deserializer)
    ///     .track()
    ///     .filter(|path| {
    ///         if path.matches(&["<root>", "forbidden"]) {
    ///             return Err(Error::custom("forbidden field"));
    ///         }
    ///         Ok(None::<()>)
    ///     })
    ///     .deserialize();
    /// assert_eq!(result.err().unwrap().to_string(), "forbidden field");
    /// ```
    pub fn filter<'de, C, Filter>(self, filter: Filter) -> Builder<D, Filter, P>
    where
        D: Deserializer<'de>,
        Filter: FnMut(&Path<'_>) -> Result<Option<C>, D::Error>,
    {
        Builder {
            deserializer: self.deserializer,
            filter: Some(filter),
            peek: self.peek,
        }
    }

    /// Sets a callback that receives an owned JSON value for selected fields.
    ///
    /// Requires the `peek` feature. A filter supplies `Some(context)`; returning
    /// `None` from the filter skips this callback. Without a filter, every unknown
    /// value is selected and the context is `None` with type `Option<()>`.
    ///
    /// The callback runs after the complete subtree is parsed. It is not called
    /// if parsing fails. Return `Ok(())` to continue or `D::Error` to reject the
    /// input. Calling this method again replaces the previous peek callback.
    ///
    /// Inspection can allocate and requires `deserialize_any` support and values
    /// representable by [`serde_json::Value`]. Duplicate object keys and some
    /// format-specific representations are not preserved.
    ///
    /// ```
    /// use serde::Deserialize;
    /// use serde_unknown::DeserializerExt;
    ///
    /// #[derive(Deserialize)]
    /// struct Config { port: u16 }
    ///
    /// let mut deserializer = serde_json::Deserializer::from_str(
    ///     r#"{"port":8080,"future":{"enabled":true},"other":false}"#,
    /// );
    /// let config: Config = (&mut deserializer)
    ///     .track()
    ///     .filter(|path| Ok(path.matches(&["<root>", "future"]).then_some("future")))
    ///     .peek(|_, context, value| {
    ///         assert_eq!(context, Some("future"));
    ///         assert_eq!(value, serde_json::json!({"enabled": true}));
    ///         Ok(())
    ///     })
    ///     .deserialize()?;
    /// deserializer.end()?;
    /// assert_eq!(config.port, 8080);
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    #[cfg(feature = "peek")]
    pub fn peek<'de, C, Peek>(self, peek: Peek) -> Builder<D, F, Peek>
    where
        D: Deserializer<'de>,
        Peek: FnMut(&Path<'_>, Option<C>, serde_json::Value) -> Result<(), D::Error>,
    {
        Builder {
            deserializer: self.deserializer,
            filter: self.filter,
            peek: Some(peek),
        }
    }
}

implement! { @builder filter ['de, D: Deserializer<'de>] (Builder<D>) -> D::Error; () }

#[cfg(feature = "peek")]
implement! { @builder peek ['de, D: Deserializer<'de>, F, C] (Builder<D, F>) -> D::Error;
    C where [F: FnMut(&Path<'_>) -> Result<Option<C>, D::Error>]
}

#[cfg(feature = "peek")]
implement! { @builder filter ['de, D: Deserializer<'de>, P] (Builder<D, (), P>) -> D::Error;
    () where [P: FnMut(&Path<'_>, Option<()>, serde_json::Value) -> Result<(), D::Error>]
}

implement! { ['de, D: Deserializer<'de>, F, P, C] (Builder<D, F, P>) -> D::Error;
    /// Deserializes `T` using the configured callbacks.
    ///
    /// Returns the first callback error unchanged, or the underlying
    /// deserializer's result when no callback rejects the input.
    pub fn deserialize<T: Deserialize<'de>>(self) -> Result<T, D::Error> {
        let mut state = State {
            filter: self.filter,
            peek: self.peek,
            error: None,
            key: None,
            index: 0,
        };
        let path = Path::Root;
        let result = T::deserialize(Tracker::new(self.deserializer, &mut state, &path));
        state.error.map_or(result, Err)
    }
}

struct State<E, F, P> {
    filter: Option<F>,
    #[cfg_attr(not(feature = "peek"), allow(dead_code))]
    peek: Option<P>,
    error: Option<E>,
    key: Option<CompactString>,
    index: usize,
}

impl<E, F, P> State<E, F, P> {
    fn check<Error: de::Error, T>(&mut self, result: Result<T, E>) -> Result<T, Error> {
        result.map_err(|error| {
            self.error.get_or_insert(error);
            Error::custom("unknown field rejected")
        })
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
struct Tracker<'s, 'p, T, E, F, P> {
    inner: T,
    #[deref(forward)]
    #[deref_mut(forward)]
    state: &'s mut State<E, F, P>,
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

implement! { ['de, 's, 'p, D: Deserializer<'de>, E, F, P, C]
    (Deserializer<'de> for Tracker<'s, 'p, D, E, F, P>) -> E;
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
        if self.state.error.is_some() {
            return Err(de::Error::custom("unknown field rejected"));
        }
        let context = match self.state.filter.as_mut() {
            Some(filter) => {
                let result = filter(self.path);
                match self.state.check(result)? {
                    Some(context) => Some(context),
                    None => return self.inner.deserialize_ignored_any(visitor),
                }
            }
            None => None,
        };
        #[cfg(feature = "peek")]
        if let Some(peek) = self.state.peek.as_mut() {
            let value = <serde_json::Value as Deserialize>::deserialize(self.inner)?;
            let result = peek(self.path, context, value);
            self.state.check(result)?;
            return visitor.visit_unit();
        }
        #[cfg(not(feature = "peek"))]
        let _ = context;
        self.inner.deserialize_ignored_any(visitor)
    }

    delegate! { @plain
        fn is_human_readable(&self) -> bool;
    }
}

implement! { ['de, 's, 'p, V: Visitor<'de>, E, F, P, C]
    (Visitor<'de> for Tracker<'s, 'p, V, E, F, P>) -> E;
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

implement! { ['de, 's, 'p, S: DeserializeSeed<'de>, E, F, P, C]
    (DeserializeSeed<'de> for Tracker<'s, 'p, S, E, F, P>) -> E;
    type Value = S::Value;
    delegate! { @wrap
        fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error>;
    }
}

implement! { ['de, 's, 'p, A: SeqAccess<'de>, E, F, P, C]
    (SeqAccess<'de> for Tracker<'s, 'p, A, E, F, P>) -> E;
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

implement! { ['de, 's, 'p, A: MapAccess<'de>, E, F, P, C]
    (MapAccess<'de> for Tracker<'s, 'p, A, E, F, P>) -> E;
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

implement! { ['de, 's, 'p, A: EnumAccess<'de>, E, F, P, C]
    (EnumAccess<'de> for Tracker<'s, 'p, A, E, F, P>) -> E;
    type Error = A::Error;
    type Variant = Tracker<'s, 'p, A::Variant, E, F, P>;
    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), A::Error> {
        let (value, access) = self.inner.variant_seed(seed)?;
        Ok((value, Tracker::new(access, self.state, self.path)))
    }
}

implement! { ['de, 's, 'p, A: VariantAccess<'de>, E, F, P, C]
    (VariantAccess<'de> for Tracker<'s, 'p, A, E, F, P>) -> E;
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
