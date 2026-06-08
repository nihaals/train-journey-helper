use std::sync::OnceLock;

use jiff::{Error, Timestamp, Zoned, civil::DateTime, tz::TimeZone};

const TIMEZONE_NAME: &str = "Europe/London";

// Use OnceLock over LazyLock so we can force the error early
static TIMEZONE: OnceLock<TimeZone> = OnceLock::new();

pub fn init() -> Result<(), Error> {
    let timezone = TimeZone::get(TIMEZONE_NAME)?;
    let _ = TIMEZONE.set(timezone);
    Ok(())
}

fn get() -> TimeZone {
    TIMEZONE
        .get()
        .expect("timezone should be initialized during startup")
        .clone()
}

pub trait TimestampExt {
    fn to_london_zoned(&self) -> Zoned;
}

impl TimestampExt for Timestamp {
    fn to_london_zoned(&self) -> Zoned {
        self.to_zoned(get())
    }
}

pub trait DateTimeExt {
    fn to_london_zoned(&self) -> Result<Zoned, Error>;
}

impl DateTimeExt for DateTime {
    fn to_london_zoned(&self) -> Result<Zoned, Error> {
        self.to_zoned(get())
    }
}
