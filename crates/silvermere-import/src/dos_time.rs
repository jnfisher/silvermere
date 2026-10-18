//! DOS packed dates and times to Unix seconds UTC.
//!
//! The records hold the kernel's `today()` date, `(year-1980)<<9 | month<<5 | day`, and
//! `now()` time, `hour<<11 | minute<<5 | second/2`. Both are the BBS's local wall clock with
//! no zone, so the importer names the zone the legacy server ran in. The schema stores Unix seconds UTC with 0 for "never",
//! which is what a zero DOS date means in the records.

use jiff::civil;
use jiff::tz::TimeZone;

/// A DOS date or time whose fields are out of range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DosTimeError {
    /// Month 0 or above 12, day 0 or past the end of the month.
    #[error("invalid DOS date {0:#06x}")]
    Date(u16),
    /// Hour above 23, minute above 59 or second above 58.
    #[error("invalid DOS time {0:#06x}")]
    Time(u16),
}

/// The moment `date` and `time` name in `tz`, as Unix seconds UTC.
///
/// A zero `date` is "never" and gives 0 whatever `time` holds. Pass `time` 0 for a field
/// that is a date only (it then means local midnight). A wall-clock time that a DST change
/// skips or repeats resolves as jiff's compatible disambiguation does (later offset for a
/// gap, earlier for a fold).
pub fn dos_to_unix(date: u16, time: u16, tz: &TimeZone) -> Result<i64, DosTimeError> {
    if date == 0 {
        return Ok(0);
    }
    let year = 1980 + i16::try_from(date >> 9).expect("7 bits");
    let month = i8::try_from((date >> 5) & 0x0F).expect("4 bits");
    let day = i8::try_from(date & 0x1F).expect("5 bits");
    let hour = i8::try_from(time >> 11).expect("5 bits");
    let minute = i8::try_from((time >> 5) & 0x3F).expect("6 bits");
    let second = i8::try_from((time & 0x1F) * 2).expect("6 bits");
    let date_v = civil::Date::new(year, month, day).map_err(|_| DosTimeError::Date(date))?;
    let time_v = civil::Time::new(hour, minute, second, 0).map_err(|_| DosTimeError::Time(time))?;
    let zoned = date_v
        .to_datetime(time_v)
        .to_zoned(tz.clone())
        .map_err(|_| DosTimeError::Date(date))?;
    Ok(zoned.timestamp().as_second())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dos_date(y: u16, m: u16, d: u16) -> u16 {
        ((y - 1980) << 9) | (m << 5) | d
    }

    fn dos_time(h: u16, m: u16, s: u16) -> u16 {
        (h << 11) | (m << 5) | (s / 2)
    }

    #[test]
    fn zero_date_is_never() {
        assert_eq!(dos_to_unix(0, 0, &TimeZone::UTC), Ok(0));
        assert_eq!(dos_to_unix(0, dos_time(12, 0, 0), &TimeZone::UTC), Ok(0));
    }

    #[test]
    fn date_and_time_in_utc() {
        // 1997-03-14 13:45:30 UTC
        let got = dos_to_unix(dos_date(1997, 3, 14), dos_time(13, 45, 30), &TimeZone::UTC);
        assert_eq!(got, Ok(858_347_130));
    }

    #[test]
    fn date_only_is_local_midnight() {
        let est = TimeZone::fixed(jiff::tz::offset(-5));
        // 1997-03-14 00:00 at UTC-5 is 05:00 UTC.
        let got = dos_to_unix(dos_date(1997, 3, 14), 0, &est);
        assert_eq!(got, Ok(858_297_600 + 5 * 3600));
    }

    #[test]
    fn range_ends() {
        let first = dos_to_unix(dos_date(1980, 1, 1), 0, &TimeZone::UTC);
        assert_eq!(first, Ok(315_532_800));
        // The last representable day, 2107-12-31 23:59:58, is past i32 seconds.
        let last = dos_to_unix(0xFF9F, 0xBF7D, &TimeZone::UTC).unwrap();
        assert_eq!(last, 4_354_819_198);
    }

    #[test]
    fn bad_fields_are_errors() {
        assert_eq!(
            dos_to_unix(dos_date(1997, 0, 14), 0, &TimeZone::UTC),
            Err(DosTimeError::Date(dos_date(1997, 0, 14)))
        );
        assert_eq!(
            dos_to_unix(dos_date(1997, 2, 30), 0, &TimeZone::UTC),
            Err(DosTimeError::Date(dos_date(1997, 2, 30)))
        );
        let t = dos_time(24, 0, 0);
        assert_eq!(
            dos_to_unix(dos_date(1997, 3, 14), t, &TimeZone::UTC),
            Err(DosTimeError::Time(t))
        );
        // second field 30 means 60 seconds
        let t = 30;
        assert_eq!(
            dos_to_unix(dos_date(1997, 3, 14), t, &TimeZone::UTC),
            Err(DosTimeError::Time(t))
        );
    }
}
