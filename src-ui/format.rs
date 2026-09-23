use wasm_bindgen::JsValue;

const BYTE_UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

pub fn bytes(value: u64) -> String {
    let mut amount = value as f64;
    let mut unit = 0;
    while amount >= 1024.0 && unit < BYTE_UNITS.len() - 1 {
        amount /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else if amount >= 100.0 {
        format!("{amount:.0} {}", BYTE_UNITS[unit])
    } else {
        format!("{amount:.1} {}", BYTE_UNITS[unit])
    }
}

pub fn speed(bytes_per_sec: f64) -> String {
    format!("{}/s", bytes(bytes_per_sec.max(0.0) as u64))
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

pub fn eta(secs: u64) -> String {
    format!("{} left", clock(secs as f64))
}

/// Coarse duration for totals, e.g. `3 h 12 min`.
pub fn long_duration(secs: f64) -> String {
    let minutes = (secs.max(0.0) / 60.0).round() as u64;
    let (h, m) = (minutes / 60, minutes % 60);
    match (h, m) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

pub fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn plural(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", count(n))
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn day_month_year(day: u32, month_index: u32, year: u32) -> String {
    let month = MONTHS
        .get(month_index as usize)
        .copied()
        .unwrap_or_else(|| wasm_bindgen::throw_str("Date returned a month outside 0 to 11"));
    format!("{day} {month} {year}")
}

fn local_date(ms: i64) -> js_sys::Date {
    js_sys::Date::new(&JsValue::from_f64(ms as f64))
}

/// `12 Sep 2026` in the local time zone. Built by hand rather than with
/// `toLocaleString`, whose month abbreviations differ between ICU versions.
pub fn date_ms(ms: i64) -> String {
    let date = local_date(ms);
    day_month_year(date.get_date(), date.get_month(), date.get_full_year())
}

/// `12 Sep 2026, 21:04` in the local time zone.
pub fn date_time_ms(ms: i64) -> String {
    let date = local_date(ms);
    format!(
        "{}, {:02}:{:02}",
        day_month_year(date.get_date(), date.get_month(), date.get_full_year()),
        date.get_hours(),
        date.get_minutes()
    )
}

/// Formats a `YYYY-MM-DD` date from the backend as `12 Sep 2026`.
pub fn iso_date(value: &str) -> Result<String, String> {
    let invalid = || format!("expected YYYY-MM-DD, got {value:?}");
    let parts: Vec<&str> = value.split('-').collect();
    let [y, m, d] = parts.as_slice() else {
        return Err(invalid());
    };
    let parse = |s: &str| s.parse::<u32>().map_err(|_| invalid());
    let (y, m, d) = (parse(y)?, parse(m)?, parse(d)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err(invalid());
    }
    Ok(day_month_year(d, m - 1, y))
}

pub const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Extension of a path from the backend, which may use either separator.
pub fn extension(path: &str) -> Option<&str> {
    let name = path.rsplit(['/', '\\']).next()?;
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then_some(ext)
}
