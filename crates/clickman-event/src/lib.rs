//! Typed ClickMan events. An application declares its events as one enum and
//! derives [`Event`](macro@Event): each variant gets its name and properties,
//! and the enum its [`Catalog`] (docs/PROTOCOL.md, "Event catalog").
//!
//! Names come from the declaration in snake case. Descriptions come from the
//! `///` comments, their lines joined with single spaces. A field's type gives
//! the property's type, and `Option` makes it optional; a property that takes
//! a fixed set of strings is an enum deriving [`Values`](macro@Values).
//!
//! ```
//! use clickman_event::{Event, Values};
//!
//! #[derive(Event)]
//! enum Studio {
//!     /// Export wrote every file
//!     /// and they are ready to use.
//!     ExportCompleted {
//!         /// File format of the export.
//!         format: Format,
//!         /// Length of the exported audio in seconds.
//!         duration_seconds: Option<f64>,
//!     },
//!     /// A new empty project was created.
//!     ProjectCreated,
//! }
//!
//! #[derive(Values)]
//! enum Format {
//!     Wav,
//!     Flac,
//! }
//!
//! let export = Studio::ExportCompleted { format: Format::Flac, duration_seconds: None };
//! assert_eq!(export.name(), "export_completed");
//! assert_eq!(export.properties()["format"], "flac");
//! assert_eq!(Studio::catalog().events.len(), 2);
//! ```
//!
//! Every event and property needs a description:
//!
//! ```compile_fail
//! #[derive(clickman_event::Event)]
//! enum Studio {
//!     ProjectCreated,
//! }
//! ```
//!
//! A description is one line without control characters:
//!
//! ```compile_fail
//! #[derive(clickman_event::Event)]
//! enum Studio {
//!     #[doc = "A new empty\u{0} project was created."]
//!     ProjectCreated,
//! }
//! ```
//!
//! A string property can carry personal data, so free text is declared:
//!
//! ```compile_fail
//! #[derive(clickman_event::Event)]
//! enum Studio {
//!     /// The project was renamed.
//!     ProjectRenamed {
//!         /// The new name.
//!         name: String,
//!     },
//! }
//! ```
//!
//! ```
//! #[derive(clickman_event::Event)]
//! enum Studio {
//!     /// The project was renamed.
//!     ProjectRenamed {
//!         /// The new name.
//!         #[clickman(text)]
//!         name: String,
//!     },
//! }
//! ```

pub use clickman_macros::{Event, Values};
#[doc(hidden)]
pub use serde_json;

use serde_json::{Map, Value, json};

/// An event declared in code: see the [crate] documentation.
pub trait Event {
    /// The event's name, such as `export_completed`.
    fn name(&self) -> &'static str;

    /// The event's properties; an optional one that is absent is left out.
    fn properties(&self) -> Map<String, Value>;

    /// The declarations of every event of this type.
    fn catalog() -> Catalog
    where
        Self: Sized;
}

/// The strings a property takes, declared as an enum of unit variants.
pub trait Values {
    /// Every value, in declaration order.
    const VALUES: &'static [&'static str];

    /// This value.
    fn value(&self) -> &'static str;
}

/// How a field's type appears in the catalog and in an event.
pub trait Property {
    /// The property's type in the catalog.
    const KIND: Kind;

    /// The value in an event.
    fn to_value(&self) -> Value;
}

/// A property's type in the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A string declared free text.
    Text,
    /// One of these strings.
    Values(&'static [&'static str]),
    Integer,
    Number,
    Boolean,
}

macro_rules! property {
    ($kind:ident: $($type:ty),*) => {
        $(impl Property for $type {
            const KIND: Kind = Kind::$kind;

            fn to_value(&self) -> Value {
                Value::from(*self)
            }
        })*
    };
}

property!(Integer: u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);
property!(Number: f32, f64);
property!(Boolean: bool);

impl Property for String {
    const KIND: Kind = Kind::Text;

    fn to_value(&self) -> Value {
        Value::from(self.as_str())
    }
}

impl Property for &str {
    const KIND: Kind = Kind::Text;

    fn to_value(&self) -> Value {
        Value::from(*self)
    }
}

/// What a source tracks: every event with its properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    pub events: Vec<EventSpec>,
}

/// An event of a [`Catalog`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub properties: Vec<PropertySpec>,
}

/// A property of an [`EventSpec`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertySpec {
    pub name: &'static str,
    pub description: &'static str,
    pub kind: Kind,
    pub required: bool,
}

impl PropertySpec {
    /// The declaration of a field of type `T`; derived code calls it.
    #[doc(hidden)]
    pub fn of<T: Property + ?Sized>(
        name: &'static str,
        description: &'static str,
        required: bool,
    ) -> Self {
        Self {
            name,
            description,
            kind: T::KIND,
            required,
        }
    }

    fn to_json(&self) -> Value {
        let mut property = Map::new();
        property.insert("name".into(), self.name.into());
        property.insert("description".into(), self.description.into());
        let kind = match self.kind {
            Kind::Text | Kind::Values(_) => "string",
            Kind::Integer => "integer",
            Kind::Number => "number",
            Kind::Boolean => "boolean",
        };
        property.insert("type".into(), kind.into());
        match self.kind {
            Kind::Values(values) => {
                property.insert("values".into(), values.into());
            }
            Kind::Text => {
                property.insert("text".into(), true.into());
            }
            Kind::Integer | Kind::Number | Kind::Boolean => {}
        }
        property.insert("required".into(), self.required.into());
        Value::Object(property)
    }
}

impl Catalog {
    /// The catalog format this crate writes.
    pub const VERSION: u32 = 1;

    /// The catalog as JSON, ready to save on the server.
    pub fn to_json(&self) -> Value {
        let events: Vec<Value> = self
            .events
            .iter()
            .map(|event| {
                let properties: Vec<Value> =
                    event.properties.iter().map(PropertySpec::to_json).collect();
                json!({
                    "name": event.name,
                    "description": event.description,
                    "properties": properties,
                })
            })
            .collect();
        json!({ "version": Self::VERSION, "events": events })
    }
}
