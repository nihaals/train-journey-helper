use std::{
    fmt::{Debug, Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Deserializer, de::Error};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Station([char; 3]);

impl FromStr for Station {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut chars = s.chars();
        let station = Self([
            chars
                .next()
                .ok_or("station code must be three characters")?,
            chars
                .next()
                .ok_or("station code must be three characters")?,
            chars
                .next()
                .ok_or("station code must be three characters")?,
        ]);
        if chars.next().is_some() {
            return Err("station code must be three characters");
        }
        if station.0.iter().all(|c| c.is_ascii_uppercase()) {
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
        for c in self.0 {
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

impl Debug for Station {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}
