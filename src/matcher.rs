//! Process matching and selection logic, shared by pgrep / pkill / pidwait / pidof.
//! Matches pgrep.c's selection rules: patterns are always extended regular expressions (ERE), with support for multiple filter conditions.

use std::time::SystemTime;

use crate::platform::ProcessInfo;
use regex::Regex;

/// The full set of selection conditions (matching pgrep's flag set). An empty Vec means the condition is disabled.
#[derive(Default)]
pub struct Selection {
    // Pattern matching (ERE)
    pub full: bool,        // -f: match the full command line
    pub ignore_case: bool, // -i
    pub exact: bool,       // -x: match the whole string
    pub inverse: bool,     // -v: invert

    // Numeric/string filters (implementable cross-platform)
    pub pids: Vec<u32>,       // -p
    pub ppids: Vec<u32>,      // -P
    pub euids: Vec<String>,   // -u (name or number)
    pub ruids: Vec<String>,   // -U
    pub terminals: Vec<String>, // -t
    pub older: Option<u64>,   // -O seconds
    pub newest: bool,         // -n
    pub oldest: bool,         // -o

    // Linux-only (these fields are None on other platforms, so the filter never matches)
    pub pgroups: Vec<u32>,    // -g
    pub groups: Vec<String>,  // -G
    pub sessions: Vec<u32>,   // -s
    pub runstates: Vec<char>, // -r
    pub cgroups: Vec<String>, // --cgroup
}

fn id_matches(list: &[String], numeric: Option<u32>, name: &str) -> bool {
    list.iter().any(|want| {
        if let Ok(n) = want.parse::<u32>() {
            Some(n) == numeric
        } else {
            want.eq_ignore_ascii_case(name)
        }
    })
}

fn tty_matches(list: &[String], tty: &Option<String>) -> bool {
    let Some(t) = tty else { return false };
    list.iter().any(|want| {
        let w = want.trim_start_matches("/dev/");
        t == w || t == &format!("tty{w}") || t == &format!("pts/{w}")
    })
}

/// Determine whether a single process satisfies all filter conditions other than the pattern.
fn passes_filters(p: &ProcessInfo, sel: &Selection) -> bool {
    if !sel.pids.is_empty() && !sel.pids.contains(&p.pid) {
        return false;
    }
    if !sel.ppids.is_empty() && !sel.ppids.contains(&p.ppid) {
        return false;
    }
    if !sel.euids.is_empty() && !id_matches(&sel.euids, p.euid, &p.user) {
        return false;
    }
    if !sel.ruids.is_empty() && !id_matches(&sel.ruids, p.ruid, &p.user) {
        return false;
    }
    if !sel.terminals.is_empty() && !tty_matches(&sel.terminals, &p.tty) {
        return false;
    }
    if let Some(secs) = sel.older {
        let age_ok = p
            .start_time
            .and_then(|st| SystemTime::now().duration_since(st).ok())
            .map(|d| d.as_secs() >= secs)
            .unwrap_or(false);
        if !age_ok {
            return false;
        }
    }
    // Linux-only
    if !sel.pgroups.is_empty() && !p.pgrp.map(|g| sel.pgroups.contains(&g)).unwrap_or(false) {
        return false;
    }
    if !sel.sessions.is_empty() && !p.sid.map(|s| sel.sessions.contains(&s)).unwrap_or(false) {
        return false;
    }
    if !sel.groups.is_empty() && !id_matches(&sel.groups, p.egid.or(p.rgid), "") {
        return false;
    }
    if !sel.runstates.is_empty() && !sel.runstates.contains(&p.state) {
        return false;
    }
    if !sel.cgroups.is_empty() {
        let hit = p
            .cgroup
            .as_ref()
            .map(|c| sel.cgroups.iter().any(|w| c.contains(w.as_str())))
            .unwrap_or(false);
        if !hit {
            return false;
        }
    }
    true
}

fn pattern_matches(p: &ProcessInfo, pattern: &str, sel: &Selection) -> bool {
    let haystack = if sel.full {
        p.cmdline.join(" ")
    } else {
        p.name.clone()
    };
    let mut re_src = String::new();
    if sel.ignore_case {
        re_src.push_str("(?i)");
    }
    if sel.exact {
        re_src.push('^');
    }
    re_src.push_str(pattern);
    if sel.exact {
        re_src.push('$');
    }
    match Regex::new(&re_src) {
        Ok(re) => re.is_match(&haystack),
        Err(_) => false,
    }
}

/// Filter the entire process list; a pattern of None means filter by conditions only.
pub fn select<'a>(
    procs: &'a [ProcessInfo],
    pattern: Option<&str>,
    sel: &Selection,
) -> Vec<&'a ProcessInfo> {
    let mut out: Vec<&ProcessInfo> = procs
        .iter()
        .filter(|p| {
            let mut hit = passes_filters(p, sel);
            if hit && let Some(pat) = pattern {
                hit = pattern_matches(p, pat, sel);
            }
            if sel.inverse { !hit } else { hit }
        })
        .collect();

    // -n / -o: among matches keep only the newest / oldest one (by start_time)
    if sel.newest || sel.oldest {
        let pick = if sel.newest {
            out.iter().copied().filter(|p| p.start_time.is_some()).max_by_key(|p| p.start_time.unwrap())
        } else {
            out.iter().copied().filter(|p| p.start_time.is_some()).min_by_key(|p| p.start_time.unwrap())
        };
        out = pick.into_iter().collect();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn proc(pid: u32, name: &str, age_secs: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            start_time: Some(SystemTime::now() - Duration::from_secs(age_secs)),
            ..Default::default()
        }
    }

    #[test]
    fn pattern_substr() {
        let ps = vec![proc(1, "bash", 10), proc(2, "cargo", 5)];
        let sel = Selection::default();
        let m = select(&ps, Some("car"), &sel);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].pid, 2);
    }

    #[test]
    fn exact_and_inverse() {
        let ps = vec![proc(1, "sh", 10), proc(2, "bash", 5)];
        let sel = Selection { exact: true, ..Default::default() };
        assert_eq!(select(&ps, Some("sh"), &sel).len(), 1);
        let sel = Selection { exact: true, inverse: true, ..Default::default() };
        assert_eq!(select(&ps, Some("sh"), &sel).len(), 1); // bash remains
    }

    #[test]
    fn newest_oldest() {
        let ps = vec![proc(1, "a", 100), proc(2, "a", 10), proc(3, "a", 50)];
        let newest = select(&ps, Some("a"), &Selection { newest: true, ..Default::default() });
        assert_eq!(newest.len(), 1);
        assert_eq!(newest[0].pid, 2); // youngest (age 10)
        let oldest = select(&ps, Some("a"), &Selection { oldest: true, ..Default::default() });
        assert_eq!(oldest[0].pid, 1); // oldest (age 100)
    }

    #[test]
    fn older_filter() {
        let ps = vec![proc(1, "a", 100), proc(2, "a", 5)];
        let sel = Selection { older: Some(60), ..Default::default() };
        let m = select(&ps, Some("a"), &sel);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].pid, 1);
    }

    #[test]
    fn pid_filter() {
        let ps = vec![proc(1, "a", 1), proc(2, "b", 1)];
        let sel = Selection { pids: vec![2], ..Default::default() };
        let m = select(&ps, None, &sel);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].pid, 2);
    }
}
