use wasm_bindgen::JsValue;
use yaydl_shared::{Locale, StatsGranularity};

use crate::i18n::texts;

const BYTE_UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

fn separators(locale: Locale) -> (char, char) {
    match locale {
        Locale::En => ('.', ','),
        Locale::De => (',', '.'),
    }
}

/// Fixed decimals with the locale's decimal and thousands separators.
pub fn number(value: f64, decimals: usize, locale: Locale) -> String {
    let (decimal_sep, thousands_sep) = separators(locale);
    let plain = format!("{:.*}", decimals, value.abs());
    let (int_part, fraction) = match plain.split_once('.') {
        Some((int_part, fraction)) => (int_part, Some(fraction)),
        None => (plain.as_str(), None),
    };
    let mut out = String::with_capacity(plain.len() + plain.len() / 3 + 1);
    if value < 0.0 && plain.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.push('-');
    }
    for (i, c) in int_part.chars().enumerate() {
        if i > 0 && (int_part.len() - i).is_multiple_of(3) {
            out.push(thousands_sep);
        }
        out.push(c);
    }
    if let Some(fraction) = fraction {
        out.push(decimal_sep);
        out.push_str(fraction);
    }
    out
}

pub fn count(value: u64, locale: Locale) -> String {
    number(value as f64, 0, locale)
}

pub fn plural(n: u64, locale: Locale, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", count(n, locale))
    }
}

pub fn bytes(value: u64, locale: Locale) -> String {
    let mut amount = value as f64;
    let mut unit = 0;
    while amount >= 1024.0 && unit < BYTE_UNITS.len() - 1 {
        amount /= 1024.0;
        unit += 1;
    }
    let decimals = if unit == 0 || amount >= 100.0 { 0 } else { 1 };
    format!("{} {}", number(amount, decimals, locale), BYTE_UNITS[unit])
}

pub fn speed(bytes_per_sec: f64, locale: Locale) -> String {
    format!("{}/s", bytes(bytes_per_sec.max(0.0) as u64, locale))
}

/// German puts a space before the percent sign, English does not.
pub fn percent(value: f64, locale: Locale) -> String {
    let n = number(value, 0, locale);
    match locale {
        Locale::En => format!("{n}%"),
        Locale::De => format!("{n}\u{a0}%"),
    }
}

/// `h:mm:ss`, or `m:ss` below an hour.
pub fn clock(secs: f64) -> String {
    let total = secs.max(0.0).round() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Coarse duration for totals, e.g. `3 h 12 min`.
pub fn long_duration(secs: f64, locale: Locale) -> String {
    let minutes = (secs.max(0.0) / 60.0).round() as u64;
    let (h, m) = (minutes / 60, minutes % 60);
    let (hour_unit, minute_unit) = match locale {
        Locale::En => ("h", "min"),
        Locale::De => ("Std.", "Min."),
    };
    match (h, m) {
        (0, m) => format!("{m} {minute_unit}"),
        (h, 0) => format!("{} {hour_unit}", count(h, locale)),
        (h, m) => format!("{} {hour_unit} {m} {minute_unit}", count(h, locale)),
    }
}

/// `month` is 1 to 12.
fn month_name(month: u32, locale: Locale) -> &'static str {
    match texts(locale)
        .months_short
        .get(month.wrapping_sub(1) as usize)
    {
        Some(name) => name,
        None => wasm_bindgen::throw_str(&format!("month {month} is outside 1 to 12")),
    }
}

/// `month` is 1 to 12. English `12 Sep 2026`, German `12. Sep. 2026`.
pub fn date(year: i32, month: u32, day: u32, locale: Locale) -> String {
    let name = month_name(month, locale);
    match locale {
        Locale::En => format!("{day} {name} {year}"),
        Locale::De => format!("{day}. {name} {year}"),
    }
}

fn local_date(ms: i64) -> js_sys::Date {
    js_sys::Date::new(&JsValue::from_f64(ms as f64))
}

/// In the local time zone. Built from the month table rather than with
/// `toLocaleString`, whose output differs between webviews and ICU versions.
pub fn date_ms(ms: i64, locale: Locale) -> String {
    let d = local_date(ms);
    date(
        d.get_full_year() as i32,
        d.get_month() + 1,
        d.get_date(),
        locale,
    )
}

pub fn date_time_ms(ms: i64, locale: Locale) -> String {
    let d = local_date(ms);
    format!(
        "{}, {:02}:{:02}",
        date(
            d.get_full_year() as i32,
            d.get_month() + 1,
            d.get_date(),
            locale
        ),
        d.get_hours(),
        d.get_minutes()
    )
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        _ => 28,
    }
}

/// Parses a `YYYY-MM-DD` date from the backend into year, month and day.
pub fn parse_iso_date(value: &str) -> Result<(i32, u32, u32), String> {
    let invalid = || format!("expected YYYY-MM-DD, got {value:?}");
    let parts: Vec<&str> = value.split('-').collect();
    let [y, m, d] = parts.as_slice() else {
        return Err(invalid());
    };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return Err(invalid());
    }
    let year = y.parse::<i32>().map_err(|_| invalid())?;
    let month = m.parse::<u32>().map_err(|_| invalid())?;
    let day = d.parse::<u32>().map_err(|_| invalid())?;
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return Err(invalid());
    }
    Ok((year, month, day))
}

pub fn iso_date(value: &str, locale: Locale) -> Result<String, String> {
    let (y, m, d) = parse_iso_date(value)?;
    Ok(date(y, m, d, locale))
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar, after Howard
/// Hinnant's `days_from_civil`.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    (year, month, day)
}

/// ISO 8601 week number. The week belongs to the year its Thursday falls in.
pub fn iso_week(year: i32, month: u32, day: u32) -> u32 {
    let days = days_from_civil(year, month, day);
    // 1970-01-01 was a Thursday, so +3 makes Monday 0.
    let weekday = (days + 3).rem_euclid(7);
    let thursday = days - weekday + 3;
    let (thursday_year, _, _) = civil_from_days(thursday);
    let ordinal = thursday - days_from_civil(thursday_year, 1, 1);
    (ordinal / 7 + 1) as u32
}

/// Axis and table label of a statistics bucket, from its `YYYY-MM-DD` start.
pub fn bucket_label(
    start_date: &str,
    granularity: StatsGranularity,
    locale: Locale,
) -> Result<String, String> {
    let (year, month, day) = parse_iso_date(start_date)?;
    let name = month_name(month, locale);
    Ok(match (granularity, locale) {
        (StatsGranularity::Day, Locale::En) => format!("{name} {day}"),
        (StatsGranularity::Day, Locale::De) => format!("{day}. {name}"),
        (StatsGranularity::Week, Locale::En) => format!("W{}", iso_week(year, month, day)),
        (StatsGranularity::Week, Locale::De) => {
            format!("KW\u{a0}{}", iso_week(year, month, day))
        }
        (StatsGranularity::Month, _) => format!("{name} {year}"),
    })
}

/// English `around 21:00`, German `gegen 21 Uhr`.
pub fn around_hour(hour: u8, locale: Locale) -> String {
    match locale {
        Locale::En => format!("around {hour:02}:00"),
        Locale::De => format!("gegen {hour} Uhr"),
    }
}

fn file_name(path: &str) -> Option<&str> {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
}

/// Extension of a path from the backend, which may use either separator.
pub fn extension(path: &str) -> Option<&str> {
    let (stem, ext) = file_name(path)?.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then_some(ext)
}

/// File name without extension, of a path that may use either separator.
pub fn file_stem(path: &str) -> Option<&str> {
    let name = file_name(path)?;
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => Some(stem),
        _ => Some(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_use_the_locale_separators() {
        assert_eq!(count(1234, Locale::En), "1,234");
        assert_eq!(count(1234, Locale::De), "1.234");
        assert_eq!(count(1_234_567, Locale::De), "1.234.567");
        assert_eq!(count(999, Locale::De), "999");
        assert_eq!(count(0, Locale::En), "0");
        assert_eq!(number(3.24, 1, Locale::De), "3,2");
        assert_eq!(number(1234.56, 1, Locale::En), "1,234.6");
        assert_eq!(number(-1234.5, 1, Locale::De), "-1.234,5");
        assert_eq!(number(-0.01, 1, Locale::En), "0.0");
    }

    #[test]
    fn byte_sizes_and_speeds() {
        assert_eq!(bytes(0, Locale::En), "0 B");
        assert_eq!(bytes(1023, Locale::De), "1.023 B");
        assert_eq!(bytes(50_017_075, Locale::En), "47.7 MiB");
        assert_eq!(bytes(50_017_075, Locale::De), "47,7 MiB");
        assert_eq!(bytes(150 * 1024 * 1024, Locale::De), "150 MiB");
        assert_eq!(speed(3.2 * 1024.0 * 1024.0, Locale::De), "3,2 MiB/s");
        assert_eq!(speed(-5.0, Locale::En), "0 B/s");
        assert_eq!(percent(42.4, Locale::En), "42%");
        assert_eq!(percent(42.4, Locale::De), "42\u{a0}%");
    }

    #[test]
    fn plurals() {
        assert_eq!(plural(1, Locale::En, "day", "days"), "1 day");
        assert_eq!(plural(2, Locale::En, "day", "days"), "2 days");
        assert_eq!(plural(0, Locale::En, "day", "days"), "0 days");
        assert_eq!(plural(1, Locale::De, "Tag", "Tage"), "1 Tag");
        assert_eq!(plural(2, Locale::De, "Tag", "Tage"), "2 Tage");
        assert_eq!(
            plural(1234, Locale::De, "Download", "Downloads"),
            "1.234 Downloads"
        );
        let t = texts(Locale::De);
        assert_eq!((t.heat_legend)(1), "1 Download in einer Stunde");
        assert_eq!((t.heat_legend)(2), "2 Downloads in einer Stunde");
        assert_eq!((t.days)(2), "2 Tage");
        assert_eq!((texts(Locale::En).days)(1), "1 day");
    }

    #[test]
    fn durations() {
        assert_eq!(clock(65.0), "1:05");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(long_duration(59.0 * 60.0, Locale::En), "59 min");
        assert_eq!(long_duration(3.0 * 3600.0, Locale::En), "3 h");
        assert_eq!(
            long_duration(3.0 * 3600.0 + 12.0 * 60.0, Locale::De),
            "3 Std. 12 Min."
        );
    }

    #[test]
    fn dates() {
        assert_eq!(date(2026, 9, 23, Locale::En), "23 Sep 2026");
        assert_eq!(date(2026, 9, 23, Locale::De), "23. Sep. 2026");
        assert_eq!(date(2026, 3, 1, Locale::De), "1. M\u{e4}rz 2026");
        assert_eq!(iso_date("2026-09-12", Locale::De).unwrap(), "12. Sep. 2026");
        for bad in [
            "2026-9-12",
            "2026-13-01",
            "2026-02-29",
            "2026-09-31",
            "x",
            "",
        ] {
            assert!(parse_iso_date(bad).is_err(), "{bad:?} should be rejected");
        }
        assert_eq!(parse_iso_date("2024-02-29").unwrap(), (2024, 2, 29));
    }

    #[test]
    fn civil_day_conversion_round_trips() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        for days in [-800_000, -1, 0, 1, 19_000, 20_719, 2_000_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn iso_weeks() {
        assert_eq!(iso_week(2026, 9, 21), 39);
        assert_eq!(iso_week(2026, 9, 23), 39);
        assert_eq!(iso_week(2025, 12, 29), 1);
        assert_eq!(iso_week(2026, 1, 1), 1);
        assert_eq!(iso_week(2021, 1, 3), 53);
        assert_eq!(iso_week(2021, 1, 4), 1);
        assert_eq!(iso_week(2020, 12, 28), 53);
        assert_eq!(iso_week(2024, 12, 30), 1);
        assert_eq!(iso_week(2027, 1, 1), 53);
    }

    #[test]
    fn bucket_labels() {
        let label = |date, g, l| bucket_label(date, g, l).unwrap();
        assert_eq!(
            label("2026-09-23", StatsGranularity::Day, Locale::En),
            "Sep 23"
        );
        assert_eq!(
            label("2026-09-23", StatsGranularity::Day, Locale::De),
            "23. Sep."
        );
        assert_eq!(
            label("2026-09-21", StatsGranularity::Week, Locale::En),
            "W39"
        );
        assert_eq!(
            label("2026-09-21", StatsGranularity::Week, Locale::De),
            "KW\u{a0}39"
        );
        assert_eq!(
            label("2026-09-01", StatsGranularity::Month, Locale::En),
            "Sep 2026"
        );
        assert_eq!(
            label("2026-09-01", StatsGranularity::Month, Locale::De),
            "Sep. 2026"
        );
        assert!(bucket_label("Sep 23", StatsGranularity::Day, Locale::En).is_err());
    }

    #[test]
    fn hours() {
        assert_eq!(around_hour(21, Locale::En), "around 21:00");
        assert_eq!(around_hour(9, Locale::En), "around 09:00");
        assert_eq!(around_hour(21, Locale::De), "gegen 21 Uhr");
    }

    #[test]
    fn paths() {
        assert_eq!(file_stem("/music/War Chant.mp3"), Some("War Chant"));
        assert_eq!(file_stem("C:\\Music\\War Chant.mp3"), Some("War Chant"));
        assert_eq!(file_stem("/music/Dr. Who [x1].m4a"), Some("Dr. Who [x1]"));
        assert_eq!(file_stem("/music/noext"), Some("noext"));
        assert_eq!(file_stem("/music/.hidden"), Some(".hidden"));
        assert_eq!(file_stem("/music/"), None);
        assert_eq!(extension("C:\\a\\b.flac"), Some("flac"));
        assert_eq!(extension("/a/.hidden"), None);
    }
}
