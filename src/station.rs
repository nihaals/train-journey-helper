use std::{
    fmt::{Debug, Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Deserializer, de::Error};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Station([u8; 3]);

impl Station {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("station codes should be valid UTF-8")
    }
}

impl FromStr for Station {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes: [u8; 3] = s
            .as_bytes()
            .try_into()
            .map_err(|_| "station code must be three characters")?;

        if bytes.iter().all(u8::is_ascii_uppercase) {
            let station = Self(bytes);
            Ok(station)
        } else {
            Err("station code must be three uppercase ASCII letters")
        }
    }
}

impl<'de> Deserialize<'de> for Station {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(D::Error::custom)
    }
}

impl Display for Station {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Debug for Station {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_returns_station_code() {
        let station = Station::from_str("ABC").unwrap();
        assert_eq!(station.as_str(), "ABC");
    }

    #[test]
    fn from_str_rejects_codes_that_are_not_three_characters() {
        assert_eq!(
            Station::from_str("AB").unwrap_err(),
            "station code must be three characters"
        );
        assert_eq!(
            Station::from_str("ABCD").unwrap_err(),
            "station code must be three characters"
        );
    }

    #[test]
    fn from_str_rejects_non_uppercase_ascii_letters() {
        assert_eq!(
            Station::from_str("AbC").unwrap_err(),
            "station code must be three uppercase ASCII letters"
        );
        assert_eq!(
            Station::from_str("A1C").unwrap_err(),
            "station code must be three uppercase ASCII letters"
        );
        assert_eq!(
            Station::from_str("AB̃C").unwrap_err(),
            "station code must be three characters"
        );
    }

    #[test]
    fn deserialize_accepts_valid_station_code() {
        let station: Station = serde_json::from_str(r#""ABC""#).unwrap();
        assert_eq!(station.as_str(), "ABC");
    }

    #[test]
    fn deserialize_rejects_invalid_station_code() {
        let error = serde_json::from_str::<Station>(r#""abc""#).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("station code must be three uppercase ASCII letters")
        );
    }

    #[test]
    fn display_formats_station_code() {
        let station = Station::from_str("ABC").unwrap();
        assert_eq!(station.to_string(), "ABC");
    }

    #[test]
    fn debug_formats_station_code() {
        let station = Station::from_str("ABC").unwrap();
        assert_eq!(format!("{station:?}"), "ABC");
    }
}
