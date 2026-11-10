//! Opaque, prefixed entity ids. Prefixes make logs and sockets debuggable;
//! nothing outside core may parse them.

use std::sync::atomic::{AtomicU64, Ordering};

pub const WORKSPACE: &str = "ws";
pub const GROUP: &str = "grp";
pub const PANE: &str = "pne";
pub const SURFACE: &str = "srf";
pub const AGENT: &str = "agt";
pub const NOTIFICATION: &str = "ntf";

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Mint an id of the form `<prefix>_<12 lowercase hex chars>`.
///
/// Uniqueness comes from a process-wide counter mixed with the epoch; it is
/// not cryptographic and does not need to be. Ids are never used for auth.
pub fn mint(prefix: &str) -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // xorshift-ish mix so sequential ids don't look sequential
    let mut x = t ^ (n.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 29;
    format!("{prefix}_{:012x}", x & 0xffff_ffff_ffff)
}

/// Validate shape only: `<prefix>_<hex>`, length-bounded.
pub fn looks_valid(id: &str) -> bool {
    let Some((prefix, rest)) = id.split_once('_') else {
        return false;
    };
    matches!(
        prefix,
        WORKSPACE | GROUP | PANE | SURFACE | AGENT | NOTIFICATION
    ) && (6..=24).contains(&rest.len())
        && rest.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn minted_ids_are_unique_and_valid() {
        let ids: HashSet<String> = (0..1000).map(|_| mint(SURFACE)).collect();
        assert_eq!(ids.len(), 1000);
        assert!(ids.iter().all(|i| looks_valid(i)));
        assert!(ids.iter().all(|i| i.starts_with("srf_")));
    }

    #[test]
    fn rejects_garbage() {
        assert!(!looks_valid(""));
        assert!(!looks_valid("srf"));
        assert!(!looks_valid("srf_"));
        assert!(!looks_valid("srf_zzzzzz"));
        assert!(!looks_valid("badprefix_abcdef"));
    }
}
