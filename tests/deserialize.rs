use compact_str::CompactString;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_unknown::{DeserializerExt, Path};
#[cfg(not(feature = "std"))]
use std::collections::BTreeMap as Map;
#[cfg(feature = "std")]
use std::collections::HashMap as Map;
use std::{borrow::Cow, collections::BTreeMap, fmt};

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

#[test]
fn paths_match_exact_layers_and_wildcards() {
    let root = Path::Root;
    let config = Path::Key(&root, CompactString::const_new("config"));
    let index = Path::Index(&config, 3);
    let field = Path::Key(&index, CompactString::const_new("deprecated_field"));

    assert!(root.matches(&["<root>"]));
    assert!(!root.matches(&[]));
    assert!(!root.matches(&["*"]));
    assert!(config.matches(&["<root>", "config"]));
    assert!(index.matches(&["<root>", "config", "3"]));
    assert!(index.matches(&["<root>", "config", "*"]));

    for pattern in [
        ["<root>", "config", "3", "deprecated_field"],
        ["<root>", "config", "*", "deprecated_field"],
        ["<root>", "*", "3", "deprecated_field"],
        ["<root>", "config", "3", "*"],
    ] {
        assert!(field.matches(&pattern));
    }
    for pattern in [
        ["<root>", "config", "2", "deprecated_field"],
        ["<root>", "config", "invalid", "deprecated_field"],
        [
            "<root>",
            "config",
            "9999999999999999999999999999999999999999",
            "deprecated_field",
        ],
        ["<root>", "other", "3", "deprecated_field"],
        ["root", "config", "3", "deprecated_field"],
        ["<root>", "config", "3", "other"],
    ] {
        assert!(!field.matches(&pattern));
    }
    assert!(!field.matches(&["config", "3", "deprecated_field"]));
    assert!(!field.matches(&["<root>", "config", "deprecated_field"]));
    assert!(!field.matches(&["<root>", "config", "3", "deprecated_field", "extra"]));
}

#[test]
fn path_matching_preserves_literal_keys() {
    let root = Path::Root;
    let config = Path::Key(&root, CompactString::const_new("config.with.dots"));
    let entry = Path::Key(&config, CompactString::const_new("items[3]"));
    let field = Path::Key(&entry, CompactString::const_new("世界"));
    assert!(field.matches(&["<root>", "config.with.dots", "items[3]", "世界"]));
    assert!(!field.matches(&["<root>", "config", "with", "dots", "items[3]", "世界"]));

    let key = Path::Key(&root, CompactString::const_new("<root>"));
    assert!(key.matches(&["<root>", "<root>"]));
    assert!(!key.matches(&["<root>"]));
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
    by_name: Map<String, Inner>,
    by_id: BTreeMap<u32, Inner>,
    len: Meters,
    kind: Kind,
    kinds: Vec<Kind>,
}

fn run<T>(json: &str) -> (T, Vec<String>)
where
    T: for<'de> Deserialize<'de> + PartialEq + fmt::Debug,
{
    let mut unknown = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let value: T = (&mut deserializer)
        .deserialize(|path| {
            unknown.push(path.to_string());
            Ok(())
        })
        .unwrap();
    deserializer.end().unwrap();
    let plain: T = serde_json::from_str(json).unwrap();
    assert_eq!(value, plain, "wrapper must not change the result");
    (value, unknown)
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
    let (_, unknown): (Outer, _) = run(json);
    assert_eq!(
        unknown,
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

    let mut unknown = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let value: Strict = (&mut deserializer)
        .deserialize(|path| {
            unknown.push(path.to_string());
            Ok(())
        })
        .unwrap();
    deserializer.end().unwrap();

    assert_eq!(value.name, "root");
    assert_eq!(value.by_name["arbitrary"].name, "map");
    assert_eq!(value.items[0].name, "item");
    assert_eq!(
        unknown,
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
fn empty_structs_report_all_fields() {
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}

    let json = r#"{"first": 1, "second": {"nested": true}}"#;
    assert!(serde_json::from_str::<Empty>(json).is_err());

    let mut unknown = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let _: Empty = (&mut deserializer)
        .deserialize(|path| {
            unknown.push(path.to_string());
            Ok(())
        })
        .unwrap();
    deserializer.end().unwrap();
    assert_eq!(unknown, ["first", "second"]);
}

#[test]
fn structs_still_accept_sequences() {
    let (value, unknown): (Inner, _) = run(r#"["name", null]"#);
    assert_eq!(value.name, "name");
    assert!(unknown.is_empty());
}

#[test]
fn struct_field_indices_and_bytes_are_forwarded() {
    fn check<'de, D: Deserializer<'de>>(deserializer: D) {
        let mut unknown = Vec::new();
        let value: Inner = deserializer
            .deserialize(|path| {
                unknown.push(path.to_string());
                Ok(())
            })
            .unwrap();
        assert_eq!(value.name, "name");
        assert!(unknown.is_empty());
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
    let (_, unknown): (Keys, _) = run(json);
    assert_eq!(
        unknown,
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
    let (_, unknown): (BTreeMap<Option<Key>, Inner>, _) = run(json);
    assert_eq!(unknown, ["first.unknown", "second.extra"]);
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
    let (_, unknown): (BTreeMap<Key, Inner>, _) = run(json);
    assert_eq!(
        unknown,
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
            <serde_json::Value as Deserialize>::deserialize(deserializer)
                .map(|value| Key(value.to_string()))
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
    let mut unknown = Vec::new();
    let deserializer = MapDeserializer::<_, serde_json::Error>::new(entries.into_iter());
    let values: BTreeMap<Key, Inner> = deserializer
        .deserialize(|path| {
            unknown.push(path.to_string());
            Ok(())
        })
        .unwrap();
    assert_eq!(values.len(), 3);
    assert_eq!(
        unknown,
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
    let (value, unknown): (Outer, _) = run(json);
    assert!(unknown.is_empty());
    assert_eq!(value.inner, None);
}

#[test]
fn nested_sequences_keep_independent_indices() {
    let json = r#"[
        [{"name":"a","x":1},{"name":"b","y":2}],
        [{"name":"c","z":3}]
    ]"#;
    let (_, unknown): (Vec<Vec<Inner>>, _) = run(json);
    assert_eq!(unknown, ["[0][0].x", "[0][1].y", "[1][0].z"]);
}

#[test]
fn unknown_value_subtrees_report_once() {
    let value = serde_json::json!({"name":"a","extra":{"deep":[1,{"x":2}]}});
    let mut unknown = Vec::new();
    let inner: Inner = value
        .deserialize(|path| {
            unknown.push(path.to_string());
            Ok(())
        })
        .unwrap();
    assert_eq!(inner.name, "a");
    assert_eq!(unknown, ["extra"]);
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
    let mut unknown = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let borrowed: BorrowedStrings = (&mut deserializer)
        .deserialize(|path| {
            unknown.push(path.to_string());
            Ok(())
        })
        .unwrap();
    assert_eq!(borrowed.text, "hello");
    assert!(matches!(borrowed.cow_text, Cow::Borrowed("world")));
    assert_eq!(unknown, ["unknown"]);
}

#[test]
fn errors_still_propagate() {
    let mut deserializer = serde_json::Deserializer::from_str(r#"{"title": 1}"#);
    let result: Result<Outer, _> = (&mut deserializer).deserialize(|_| Ok(()));
    assert!(result.is_err());
}

#[test]
fn callback_errors_stop_before_unknown_values_are_consumed() {
    let json = r#"[
        {"name":"a","allowed":{"nested":true}},
        {"name":"b","blocked":invalid},
        {"name":"c","later":0}
    ]"#;
    let mut unknown = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let result: Result<Vec<Option<Inner>>, _> = (&mut deserializer).deserialize(|path| {
        unknown.push(path.to_string());
        if path.matches(&["<root>", "1", "blocked"]) {
            return Err(de::Error::custom("blocked field"));
        }
        Ok(())
    });

    assert_eq!(result.unwrap_err().to_string(), "blocked field");
    assert_eq!(unknown, ["[0].allowed", "[1].blocked"]);
}

#[test]
fn builder_without_callbacks_accepts_unknown_fields() {
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Strict {
        name: String,
    }

    let mut deserializer = serde_json::Deserializer::from_str(r#"{"name":"a","unknown":1e400}"#);
    let value: Strict = (&mut deserializer).track().deserialize().unwrap();

    assert_eq!(value.name, "a");
    deserializer.end().unwrap();
}

#[test]
fn filter_without_peek_does_not_materialize_selected_values() {
    let mut deserializer = serde_json::Deserializer::from_str(r#"{"name":"a","unknown":1e400}"#);
    let mut paths = Vec::new();
    let value: Inner = (&mut deserializer)
        .track()
        .filter(|path| {
            paths.push(path.to_string());
            Ok(Some(()))
        })
        .deserialize()
        .unwrap();

    assert_eq!(value.name, "a");
    assert_eq!(paths, ["unknown"]);
    deserializer.end().unwrap();
}

#[test]
fn filter_errors_stop_before_values_are_consumed() {
    let mut deserializer = serde_json::Deserializer::from_str(r#"{"blocked":invalid,"later":0}"#);
    let mut callbacks = 0;
    let result: Result<Inner, _> = (&mut deserializer)
        .track()
        .filter(|path| {
            callbacks += 1;
            assert!(path.matches(&["<root>", "blocked"]));
            Err::<Option<()>, _>(de::Error::custom("blocked field"))
        })
        .deserialize();

    assert_eq!(result.unwrap_err().to_string(), "blocked field");
    assert_eq!(callbacks, 1);
}

#[derive(Debug, PartialEq, Eq)]
enum Failure {
    Callback(usize),
    Deserializer,
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Callback(code) => write!(formatter, "callback error {code}"),
            Self::Deserializer => formatter.write_str("deserializer error"),
        }
    }
}

impl core::error::Error for Failure {}

impl de::Error for Failure {
    fn custom<T: fmt::Display>(_: T) -> Self {
        Self::Deserializer
    }
}

#[test]
fn callback_errors_preserve_the_original_error() {
    let deserializer =
        de::value::MapDeserializer::<_, Failure>::new([("unknown", 0u8)].into_iter());
    let result: Result<Inner, _> = deserializer.deserialize(|_| Err(Failure::Callback(7)));

    assert_eq!(result.unwrap_err(), Failure::Callback(7));
}

struct Nested<D>(D);

impl<'de, D: Deserializer<'de>> Deserializer<'de> for Nested<D> {
    type Error = Failure;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor
            .visit_some(self.0)
            .map_err(|_| Failure::Deserializer)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

#[test]
fn callback_errors_cross_different_deserializer_error_types() {
    let mut deserializer = serde_json::Deserializer::from_str(r#"{"unknown":0}"#);
    let result: Result<Option<Inner>, _> = Nested(&mut deserializer).deserialize(|path| {
        assert!(path.matches(&["<root>", "unknown"]));
        Err(Failure::Callback(9))
    });

    assert_eq!(result.unwrap_err(), Failure::Callback(9));
}

#[test]
fn filter_errors_cross_different_deserializer_error_types() {
    let mut deserializer = serde_json::Deserializer::from_str(r#"{"unknown":invalid}"#);
    let result: Result<Option<Inner>, _> = Nested(&mut deserializer)
        .track()
        .filter(|path| {
            assert!(path.matches(&["<root>", "unknown"]));
            Err::<Option<()>, _>(Failure::Callback(11))
        })
        .deserialize();

    assert_eq!(result.unwrap_err(), Failure::Callback(11));
}

#[cfg(feature = "peek")]
#[test]
fn peek_errors_preserve_the_original_error() {
    let deserializer =
        de::value::MapDeserializer::<_, Failure>::new([("unknown", 0u8)].into_iter());
    let result: Result<Inner, _> = deserializer
        .track()
        .peek(|path, _, value| {
            assert!(path.matches(&["<root>", "unknown"]));
            assert_eq!(value, serde_json::json!(0));
            Err(Failure::Callback(13))
        })
        .deserialize();

    assert_eq!(result.unwrap_err(), Failure::Callback(13));
}

#[cfg(feature = "peek")]
#[test]
fn peek_errors_cross_different_deserializer_error_types() {
    let mut deserializer = serde_json::Deserializer::from_str(r#"{"unknown":0}"#);
    let result: Result<Option<Inner>, _> = Nested(&mut deserializer)
        .track()
        .peek(|path, _, value| {
            assert!(path.matches(&["<root>", "unknown"]));
            assert_eq!(value, serde_json::json!(0));
            Err(Failure::Callback(15))
        })
        .deserialize();

    assert_eq!(result.unwrap_err(), Failure::Callback(15));
}

#[test]
fn caught_errors_do_not_resume_callbacks_or_replace_the_first_error() {
    #[derive(Debug)]
    struct Recovery;

    impl<'de> Deserialize<'de> for Recovery {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct RecoveryVisitor;

            impl<'de> Visitor<'de> for RecoveryVisitor {
                type Value = Recovery;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("a map")
                }

                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                    while map.next_key::<String>()?.is_some() {
                        let _ = map.next_value::<de::IgnoredAny>();
                    }
                    Ok(Recovery)
                }
            }

            deserializer.deserialize_map(RecoveryVisitor)
        }
    }

    let deserializer = de::value::MapDeserializer::<_, Failure>::new(
        [("first", 0u8), ("second", 1u8)].into_iter(),
    );
    let mut callbacks = 0;
    let result: Result<Recovery, _> = deserializer.deserialize(|_| {
        callbacks += 1;
        Err(Failure::Callback(callbacks))
    });

    assert_eq!(result.unwrap_err(), Failure::Callback(1));
    assert_eq!(callbacks, 1);

    #[cfg(feature = "peek")]
    {
        let deserializer = de::value::MapDeserializer::<_, Failure>::new(
            [("first", 0u8), ("second", 1u8)].into_iter(),
        );
        let mut filters = 0;
        let mut peeks = 0;
        let result: Result<Recovery, _> = deserializer
            .track()
            .filter(|_| {
                filters += 1;
                Ok(Some(()))
            })
            .peek(|_, _, _| {
                peeks += 1;
                Err(Failure::Callback(2))
            })
            .deserialize();

        assert_eq!(result.unwrap_err(), Failure::Callback(2));
        assert_eq!(filters, 1);
        assert_eq!(peeks, 1);
    }
}
