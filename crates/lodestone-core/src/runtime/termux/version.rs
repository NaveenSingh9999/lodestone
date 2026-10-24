//! Debian version-string comparison, ported from dpkg's `versionrevision()`
//! semantics. Only the parts apt repositories actually need are supported
//! (epoch:upstream-revision with the standard comparison operators).

/// The dpkg `order()` function: digits rank first in non-numeric slots,
/// letters next, `~` before everything, non-alphanumerics last.
fn order(c: u8) -> i64 {
    if c.is_ascii_digit() {
        0
    } else if c.is_ascii_alphabetic() {
        c as i64 // letters sort in alphabetical (byte) order
    } else if c == b'~' {
        -1
    } else if c == 0 {
        0 // end of string
    } else {
        c as i64 + 256 // non-alphanumeric sorts after all letters
    }
}

fn byte_or_zero(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// Central comparison of two strings, digit runs compared numerically with
/// dpkg's zero-skipping.
fn dpkg_parts_cmp(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut i, mut j) = (0usize, 0usize);
    loop {
        let ca = byte_or_zero(a, i);
        let cb = byte_or_zero(b, j);
        if ca == 0 && cb == 0 {
            return Ordering::Equal;
        }

        if ca != 0 && cb != 0 && ca.is_ascii_digit() && cb.is_ascii_digit() {
            // numeric run with zero-skip
            let mut i2 = i;
            while i2 < a.len() && a[i2] == b'0' {
                i2 += 1;
            }
            let mut j2 = j;
            while j2 < b.len() && b[j2] == b'0' {
                j2 += 1;
            }
            let mut i3 = i2;
            let mut j3 = j2;
            while i3 < a.len() && a[i3].is_ascii_digit() {
                i3 += 1;
            }
            while j3 < b.len() && b[j3].is_ascii_digit() {
                j3 += 1;
            }
            let (da, db) = (i3 - i2, j3 - j2);
            if da != db {
                return da.cmp(&db);
            }
            let ord = a[i2..i3].cmp(&b[j2..j3]);
            if !ord.is_eq() {
                return ord;
            }
            i = i3;
            j = j3;
        } else {
            let oa = order(ca);
            let ob = order(cb);
            if oa < ob {
                return Ordering::Less;
            }
            if oa > ob {
                return Ordering::Greater;
            }
            // equal ranks but different bytes for non-special classes still
            // resolve by byte value (letters / punctuation may share no rank)
            if ca != cb {
                if oa < 0 {
                    // both '~': equal at this position, advance
                } else {
                    return ca.cmp(&cb);
                }
            }
            i += 1;
            j += 1;
        }
    }
}

/// Splits `epoch:upstream-revision`; epoch defaults to 0. Returns the numeric
/// epoch and the remainder including the revision.
fn split_epoch(v: &str) -> (u64, &str) {
    match v.split_once(':') {
        Some((e, rest)) if !e.is_empty() && e.chars().all(|c| c.is_ascii_digit()) => {
            match e.parse::<u64>() {
                Ok(epoch) => (epoch, rest),
                Err(_) => (0, v),
            }
        }
        _ => (0, v),
    }
}

/// Consistent with dpkg's string compare of the epoch-less part.
fn epoch_str(v: &str) -> u64 {
    split_epoch(v).0
}

fn after_epoch(v: &str) -> &str {
    split_epoch(v).1
}

/// Splits `upstream-revision` into upstream and revision on the last '-'.
fn split_upstream(v: &str) -> (&str, &str) {
    match v.rsplit_once('-') {
        Some((up, rev)) => (up, rev),
        None => (v, ""),
    }
}

pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (ea, eb) = (epoch_str(a), epoch_str(b));
    if ea != eb {
        return ea.cmp(&eb);
    }
    let (ua, ra) = split_upstream(after_epoch(a));
    let (ub, rb) = split_upstream(after_epoch(b));
    let up = dpkg_parts_cmp(ua.as_bytes(), ub.as_bytes());
    if !up.is_eq() {
        return up;
    }
    dpkg_parts_cmp(ra.as_bytes(), rb.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::version_cmp;
    use std::cmp::Ordering;

    fn c(a: &str, b: &str) -> Ordering {
        version_cmp(a, b)
    }

    #[test]
    fn equal() {
        assert_eq!(c("21.0.13", "21.0.13"), Ordering::Equal);
        assert_eq!(c("1:2026.10.20-1", "1:2026.10.20-1"), Ordering::Equal);
    }

    #[test]
    fn upstream() {
        assert!(c("21.0.12", "21.0.13") == Ordering::Less);
        assert!(c("9.9", "10.0") == Ordering::Less);
        assert!(c("10", "9") == Ordering::Greater);
        // zero skipping
        assert_eq!(c("1.010", "1.10"), Ordering::Equal);
        // tilde sorts before end
        assert!(c("1.0~rc1", "1.0") == Ordering::Less);
        // letters after end-of-string
        assert!(c("1.0", "1.0a") == Ordering::Less);
    }

    #[test]
    fn epochs_and_revisions() {
        assert!(c("1:0", "2:0") == Ordering::Less);
        assert!(c("2:0", "10:0") == Ordering::Less);
        assert!(c("1:2026.10.20-1", "1:2025.01.01-9") == Ordering::Greater);
        assert_eq!(c("1.0-1", "1.0-2"), Ordering::Less);
        assert!(c("1.0-12", "1.0-2") == Ordering::Greater);
        // no epoch = 0
        assert!(c("0.5", "1:0.1") == Ordering::Less);
    }
}
