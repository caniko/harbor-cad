//! Decode tagged JSON fields directly, independent of serde_json feature unions.

use serde::Deserializer;
use serde::de::{MapAccess, Visitor};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::fmt;

pub(crate) fn object<'de, D>(deserializer: D) -> Result<BTreeMap<String, Box<RawValue>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct Object;
    impl<'de> Visitor<'de> for Object {
        type Value = BTreeMap<String, Box<RawValue>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a tagged JSON object with distinct fields")
        }

        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut fields = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                if fields.contains_key(&key) {
                    return Err(serde::de::Error::custom(format!("duplicate field {key}")));
                }
                fields.insert(key, map.next_value()?);
            }
            Ok(fields)
        }
    }
    deserializer.deserialize_map(Object)
}

pub(crate) fn snake_case(variant: &str) -> String {
    let mut result = String::new();
    for (index, character) in variant.chars().enumerate() {
        if character.is_uppercase() && index != 0 {
            result.push('_');
        }
        result.extend(character.to_lowercase());
    }
    result
}

// Serialization and schema generation retain the declared internally tagged
// representation. Decoding selects a variant before asking its ordinary struct
// visitor to read typed fields: serde's Content buffer cannot retain decimal
// numbers under arbitrary_precision. RawValue keeps nested duplicate and unknown
// fields visible to the concrete deserializer, too.
macro_rules! tagged_enum {
    (
        $tag:literal;
        $(#[$attribute:meta])*
        pub enum $name:ident {
            $(
                $variant:ident {
                    $( $(#[$field_attribute:meta])* $field:ident: $field_type:ty ),* $(,)?
                }
            ),* $(,)?
        }
    ) => {
        $(#[$attribute])*
        pub enum $name {
            $( $variant { $( $(#[$field_attribute])* $field: $field_type ),* } ),*
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
                let mut fields = $crate::json::object(deserializer)?;
                let tag = fields.remove($tag)
                    .ok_or_else(|| serde::de::Error::missing_field($tag))?;
                let tag: String = serde_json::from_str(tag.get())
                    .map_err(serde::de::Error::custom)?;
                let payload = serde_json::to_string(&fields)
                    .map_err(serde::de::Error::custom)?;
                $(
                    if tag == $crate::json::snake_case(stringify!($variant)) {
                        #[derive(serde::Deserialize)]
                        #[serde(deny_unknown_fields)]
                        struct Fields {
                            $( $(#[$field_attribute])* $field: $field_type ),*
                        }
                        let Fields { $( $field ),* } = serde_json::from_str(&payload)
                            .map_err(serde::de::Error::custom)?;
                        return Ok(Self::$variant { $( $field ),* });
                    }
                )*
                Err(serde::de::Error::custom(format!("unknown {} variant {tag}", $tag)))
            }
        }
    };
}

pub(crate) use tagged_enum;
