/// Accepts "1d12h", "90m", "45s", "2w" or a bare number meaning minutes
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(n) = s.parse::<i64>() {
        return n.checked_mul(60).filter(|v| *v > 0);
    }

    let mut total: i64 = 0;
    let mut num = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
            continue;
        }
        let n: i64 = num.parse().ok()?;
        num.clear();
        let mult = match ch.to_ascii_lowercase() {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86_400,
            'w' => 604_800,
            _ => return None,
        };
        total = total.checked_add(n.checked_mul(mult)?)?;
    }
    if !num.is_empty() || total <= 0 {
        return None;
    }
    Some(total)
}

pub fn fmt_duration(mut secs: i64) -> String {
    if secs <= 0 {
        return "0s".into();
    }
    let mut out = String::new();
    for (unit, size) in [("d", 86_400), ("h", 3600), ("m", 60), ("s", 1)] {
        let n = secs / size;
        if n > 0 {
            out.push_str(&format!("{n}{unit}"));
            secs -= n * size;
        }
    }
    out
}

pub fn clamp_reason(r: &str) -> String {
    r.chars().take(512).collect()
}

/// Embed field values cap at 1024
pub fn clamp_field(s: &str) -> String {
    if s.chars().count() <= 1024 {
        return s.to_string();
    }
    s.chars().take(1021).chain("...".chars()).collect()
}

/// Embed descriptions cap at 4096
pub fn clamp_desc(s: &str) -> String {
    if s.chars().count() <= 4096 {
        return s.to_string();
    }
    s.chars().take(4093).chain("...".chars()).collect()
}
