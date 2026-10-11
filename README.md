# serde-unknown

Handle unknown fields during Serde deserialization, including struct
fields that would normally be rejected by `deny_unknown_fields` or pbjson.
The callback receives the field path; the caller chooses how to handle it.
Use `DeserializerExt::deserialize` for a path callback, or
`DeserializerExt::track` to configure a builder.

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
        if path.matches(&["<root>", "future"]) {
            eprintln!("Unknown field: {path}");
        }
        Ok(())
    })?;
    deserializer.end()?;
    assert_eq!(config.port, 8080);
    Ok(())
}
```

The callback returns `Result<(), D::Error>`. Return `Ok(())` to continue, or
return the deserializer's native error to reject the input:

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

The callback runs before the adapter delegates consumption of the unknown value.
The first callback error is saved and returned unchanged from the outermost call,
even if a nested deserializer uses a different error type. Nested code can catch
the intermediate error and continue executing, but the outermost call still
returns the saved error. Subsequent callbacks are not invoked after rejection.
Saving the error requires no heap allocation. Intermediate deserializers may
construct errors to stop processing, and error construction can allocate.

## Builder and value inspection

`track()` returns a `Builder<D, (), ()>` with both callbacks absent. Its
`filter` callback returns `Result<Option<C>, D::Error>` and runs before each
unknown value is consumed. Return `Some(context)` to select the value and pass
its classification to `peek`, `None` to let the underlying deserializer consume
it directly, or an error to reject the input. The classification can be an enum,
an index, or another caller-defined value.

With the `peek` feature enabled, `peek` receives the path, an `Option<C>`, and
an owned `serde_json::Value`, and returns `Result<(), D::Error>`. A configured
filter supplies `Some(context)`; returning `None` from the filter skips `peek`
entirely. Without a filter, every unknown value is selected and `peek` receives
`None`. Without a `peek` callback, the filter still runs, but values are consumed
directly regardless of its result. With neither callback configured, no callback
runs. The callbacks use static dispatch; the builder and passing classifications
require no heap allocation.

```rust
# #[cfg(feature = "peek")]
# fn main() -> Result<(), serde_json::Error> {
use serde::{Deserialize, de::Error};
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

let mut deserializer = serde_json::Deserializer::from_str(
    r#"{"port":8080,"future":{"nested":true},"legacy":[1,2],"other":false}"#,
);
let config: Config = (&mut deserializer)
    .track()
    .filter(|path| {
        if path.matches(&["<root>", "forbidden"]) {
            return Err(Error::custom("forbidden field"));
        }
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
# Ok(())
# }
# #[cfg(not(feature = "peek"))]
# fn main() {}
```

Paths are matched only in the filter; `peek` uses the supplied classification
without matching them again.

Both callbacks preserve the first error unchanged, using the same behavior as
`DeserializerExt::deserialize`. The filter can reject input before parsing a
value. The peek callback runs after the entire selected value has been parsed;
if parsing fails, that callback is not called. Each unknown subtree produces
one peek callback, rather than callbacks for its nested contents.

Value inspection uses `serde_json::Value::deserialize`, which requires the
underlying format to support `deserialize_any`. It can allocate and does not
preserve every format-specific value or duplicate object key. Enabling the
feature alone does not materialize values; only a configured, selected peek does.

Paths use a stack-linked list. Keys use `CompactString`, so short keys stay inline
while long keys may allocate. Known fields still reject invalid values and
duplicates. Map keys are unrestricted. Flattened fields are not tracked.

`Path::matches` compares the complete path without formatting or allocation.
Patterns run from root to leaf and start with `"<root>"`. `"*"` matches one key
or sequence index; indices use decimal numbers, such as `"3"`. For example,
`path.matches(&["<root>", "config", "*", "deprecated_field"])` matches the field
under one entry in `config`. Keys containing dots or brackets remain one segment.
Matching takes linear time in the compared segments and constant auxiliary space.

## Features

The `std` feature is enabled by default. The `peek` feature is optional and
disabled by default. Enable it to use `Builder::peek`:

```toml
serde-unknown = { version = "0.1", features = ["peek"] }
```

Disable default features to use
`serde-unknown` in a `no_std` environment:

```toml
serde-unknown = { version = "0.1", default-features = false }
```

This configuration still requires `alloc` and an allocator in the final
application. The `peek` feature also works without `std`. Short keys stay inline;
long keys may allocate. `Path` implements `Display` and `Debug` with or without `std`.
