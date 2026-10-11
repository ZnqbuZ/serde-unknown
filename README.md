# serde-unknown

Handle unknown fields during Serde deserialization: log their paths, reject
them with a native deserializer error, or inspect selected values.

Works with ordinary Serde structs, `#[serde(deny_unknown_fields)]`, and
pbjson-generated messages. Callbacks use static dispatch, and paths are tracked
with a stack-linked list.

[crates.io](https://crates.io/crates/serde-unknown) ·
[API documentation](https://docs.rs/serde-unknown) ·
[Source](https://github.com/ZnqbuZ/serde-unknown)

## Installation

```toml
[dependencies]
serde-unknown = "0.1"
```

The JSON examples below also use:

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

## Path callbacks

Import `DeserializerExt` and call `.deserialize(callback)` on a deserializer.
Return `Ok(())` to continue or `Err(...)` to reject the input.

```rust
use serde::Deserialize;
use serde_unknown::DeserializerExt;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    port: u16,
}

fn main() -> Result<(), serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(
        r#"{"port":8080,"future":{"nested":true}}"#,
    );
    let config: Config = (&mut deserializer).deserialize(|path| {
        eprintln!("Unknown field: {path}");
        Ok(())
    })?;
    deserializer.end()?;

    assert_eq!(config.port, 8080);
    Ok(())
}
```

This prints `Unknown field: future`. An unknown subtree produces one callback
for its outer field; its contents are consumed by the underlying deserializer.
Known fields retain the target type's validation and duplicate-field behavior.

Use a deserializer directly instead of a convenience function such as
`serde_json::from_str`. For pbjson, replace `Config` with your generated message
type; the generated code does not need to change. The adapter applies to the
individual deserialization call. `deserializer.end()` checks that a complete
JSON document has no trailing input.

## Rejecting input

The callback returns `Result<(), D::Error>`, using the underlying deserializer's
error type. `serde::de::Error::custom` constructs that error without an additional
public error wrapper:

```rust
use serde::{Deserialize, de::Error};
use serde_unknown::DeserializerExt;

#[derive(Deserialize)]
struct Config {
    port: u16,
}

let mut deserializer = serde_json::Deserializer::from_str(
    r#"{"port":8080,"forbidden":{"nested":true}}"#,
);
let result: Result<Config, _> = (&mut deserializer).deserialize(|path| {
    if path.matches(&["<root>", "forbidden"]) {
        return Err(Error::custom("forbidden field"));
    }
    Ok(())
});
assert_eq!(result.err().unwrap().to_string(), "forbidden field");
```

Path callbacks run before consuming the unknown value. The first callback error
is returned unchanged, including when nested deserializers use a different error
type or a custom `Deserialize` implementation catches the intermediate error.
After rejection, no further callbacks run. Keeping the original error does not
allocate; constructing an error may allocate.

## Matching paths

`Path::matches` compares complete paths without formatting or allocating.
Patterns start with `"<root>"` and proceed from root to leaf. `"*"` matches one
key or sequence index, and indices can be matched with decimal strings.

| Displayed path | Matching pattern |
| --- | --- |
| `<root>` | `["<root>"]` |
| `future` | `["<root>", "future"]` |
| `items[3].extra` | `["<root>", "items", "3", "extra"]` |
| Any `items` entry's `extra` field | `["<root>", "items", "*", "extra"]` |
| `[0].extra` | `["<root>", "0", "extra"]` |

Display omits the root prefix for non-root paths. Keys containing dots or
brackets are still single segments when matching; the display form does not
escape them. `"*"` always acts as a wildcard and has no escape syntax. Matching
takes linear time in the compared segments and constant auxiliary space.

Paths borrow their parents on the stack and are valid during the callback.
Format them into a string if you need to keep them afterward.

## Filtering and inspecting values

Enable `peek` to receive selected unknown values as owned `serde_json::Value`s:

```toml
serde-unknown = { version = "0.1", features = ["peek"] }
```

Call `.track()` to configure a builder. The filter returns
`Result<Option<C>, D::Error>`: `Some(context)` selects a value, `None` skips
inspection, and an error rejects the input. The context can be an enum, index,
or another value of your choice. `peek` receives that context along with the
path and the parsed value, so paths only need to be matched once.

```rust,ignore
use serde::Deserialize;
use serde_unknown::DeserializerExt;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    port: u16,
}

enum Field {
    Future,
    Legacy,
}

fn main() -> Result<(), serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(
        r#"{"port":8080,"future":{"nested":true},"legacy":[1,2],"other":false}"#,
    );
    let config: Config = (&mut deserializer)
        .track()
        .filter(|path| {
            if path.matches(&["<root>", "future"]) {
                Ok(Some(Field::Future))
            } else if path.matches(&["<root>", "legacy"]) {
                Ok(Some(Field::Legacy))
            } else {
                Ok(None)
            }
        })
        .peek(|_, field, value| {
            match field {
                Some(Field::Future) => assert_eq!(value, serde_json::json!({"nested": true})),
                Some(Field::Legacy) => assert_eq!(value, serde_json::json!([1, 2])),
                None => unreachable!(),
            }
            Ok(())
        })
        .deserialize()?;
    deserializer.end()?;

    assert_eq!(config.port, 8080);
    Ok(())
}
```

`other` is consumed without constructing a JSON value. Both callbacks are
optional, and `.filter()` and `.peek()` can be configured in either order.

| Configuration | Behavior |
| --- | --- |
| Neither callback | Consume unknown values without calling user code. |
| Filter only | Run the filter; consume values directly unless it returns an error. |
| Peek only | Inspect every unknown value; the context is `None` with type `Option<()>`. |
| Filter and peek | Inspect only values selected by the filter; the context is `Some(C)`. |

The filter runs before a value is consumed. `peek` runs after the entire selected
subtree has been parsed and takes ownership of the resulting value. If parsing
fails, `peek` is not called. It can also return `D::Error`; the first callback
error is preserved in the same way as with `.deserialize(callback)`.

## Features and allocation

| Feature | Default | Purpose |
| --- | --- | --- |
| `std` | Enabled | Enable standard library support in dependencies. |
| `peek` | Disabled | Provide `Builder::peek` and depend on `serde_json`. |

Disable default features for `no_std` with `alloc`:

```toml
serde-unknown = { version = "0.1", default-features = false }
```

The final application needs an allocator. `peek` also works without `std`.
`Path` supports `Display` and `Debug` in both configurations.

The adapter stores path nodes on the stack and uses `CompactString` for keys.
Short keys fit inline; long keys may allocate. Matching paths and storing
callbacks add no heap allocations. Allocations made by the deserializer,
target type, callbacks, and callback contexts depend on their implementations.
Converting a path to a string or materializing a JSON value can allocate.
Enabling `peek` alone does not materialize values.

## Compatibility

Unknown-field handling uses Serde's struct field metadata and
`deserialize_ignored_any`. Arbitrary map keys are valid entries, while unknown
fields inside their struct values are tracked. `#[serde(flatten)]` is not
supported for unknown-field tracking.

Value inspection requires the underlying format to support `deserialize_any`
and represent the selected value in the JSON data model. Formats that require
an expected type cannot provide this operation.
`serde_json::Value` does not retain duplicate object keys or every
format-specific representation.

The adapter adds no depth or size limits; resource limits depend on the
underlying deserializer and the application.

## License

Licensed under either [MIT](https://github.com/ZnqbuZ/serde-unknown/blob/master/LICENSE)
or [Apache-2.0](https://github.com/ZnqbuZ/serde-unknown/blob/master/LICENSE-APACHE),
at your option.
