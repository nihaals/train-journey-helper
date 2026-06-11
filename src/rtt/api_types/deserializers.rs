use std::str::FromStr;

use serde::{Deserialize, Deserializer};

use crate::timezone::DateTimeExt;

pub(super) fn deserialize_optional_timestamp<'de, D>(
    deserializer: D,
) -> Result<Option<jiff::Timestamp>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|value| {
            jiff::civil::DateTime::from_str(&value)
                .map_err(serde::de::Error::custom)?
                .to_london_zoned()
                .map_err(serde::de::Error::custom)
                .map(|zoned| zoned.timestamp())
        })
        .transpose()
}
