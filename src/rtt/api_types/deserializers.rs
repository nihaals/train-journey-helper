use std::str::FromStr;

use jiff::{Timestamp, civil::DateTime};
use serde::{Deserialize, Deserializer};

use crate::timezone::DateTimeExt;

fn str_to_date_time<E>(s: &str) -> Result<Timestamp, E>
where
    E: serde::de::Error,
{
    DateTime::from_str(s)
        .map_err(serde::de::Error::custom)?
        .to_london_zoned()
        .map_err(serde::de::Error::custom)
        .map(|zoned| zoned.timestamp())
}

pub(super) fn deserialize_optional_timestamp<'de, D>(
    deserializer: D,
) -> Result<Option<Timestamp>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    value.as_deref().map(str_to_date_time).transpose()
}
