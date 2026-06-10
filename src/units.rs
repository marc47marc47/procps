//! Numeric formatting helpers: human-readable sizes (matching the scaling behavior of procps `free -h` / `top`).

/// Scale in base 1024, procps style (Ki/Mi/Gi..., `free -h`).
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "Ki", "Mi", "Gi", "Ti", "Pi", "Ei"];
    if bytes < 1024 {
        return format!("{bytes}{}", UNITS[0]);
    }
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 100.0 {
        format!("{:.0}{}", v, UNITS[i])
    } else if v >= 10.0 {
        format!("{:.1}{}", v, UNITS[i])
    } else {
        format!("{:.1}{}", v, UNITS[i])
    }
}

/// Convert seconds to procps `uptime` style: `3 days,  4:05` / `1:23` / `5 min`.
pub fn format_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    let mut out = String::new();
    if days > 0 {
        out.push_str(&format!("{} day{}, ", days, if days == 1 { "" } else { "s" }));
    }
    if hours > 0 || days > 0 {
        out.push_str(&format!("{hours:2}:{mins:02}"));
    } else {
        out.push_str(&format!("{mins} min"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human() {
        assert_eq!(human_bytes(512), "512B");
        assert_eq!(human_bytes(2048), "2.0Ki");
    }

    #[test]
    fn uptime_fmt() {
        assert_eq!(format_uptime(300), "5 min");
        assert_eq!(format_uptime(90_061), "1 day,  1:01");
    }
}
