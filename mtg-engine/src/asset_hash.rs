//! Content-addressed asset hashing — the SINGLE source of truth for the
//! content-addressed (immutable) web-asset pipeline (mtg-571).
//!
//! Both content-addressed asset classes hash their bytes through the SAME
//! function here, so a per-set data bin (`<YYYY>-<CODE>.<hash>.bin`, named by
//! the Rust exporter) and the wasm-bindgen pkg pair (`mtg_forge_rs.<hash>.js`
//! / `mtg_forge_rs_bg.<hash>.wasm`, named by `scripts/hash_web_assets.sh` via
//! the `mtg hash-asset` subcommand) are hashed identically. DRY: there is no
//! second hash implementation anywhere — the shell script shells out to this
//! code rather than reimplementing a hash in bash.
//!
//! ## Algorithm
//!
//! [blake3](https://github.com/BLAKE3-team/BLAKE3) truncated to the first 16
//! hex chars (64 bits). blake3 is fast, has no per-process seed (so the same
//! bytes always produce the same name across builds, Rust versions, and
//! machines — unlike `std`'s `DefaultHasher`/SipHash, which std does not
//! guarantee stable across versions), and is a single small dependency. The
//! only requirement for cache-busting is "different bytes -> different name
//! with overwhelming probability"; 64 bits gives a birthday bound of ~2^-44
//! for ~600 assets, which is ample.

/// Number of hex characters (and thus bytes/2) of the blake3 digest embedded
/// in a content-addressed filename. 16 hex chars = 64 bits.
pub const ASSET_HASH_HEX_LEN: usize = 16;

/// Hash `bytes` and return the first [`ASSET_HASH_HEX_LEN`] hex chars of the
/// blake3 digest. This is the one function that names every content-addressed
/// asset in the pipeline.
pub fn asset_hash_hex(bytes: &[u8]) -> String {
    let digest = blake3::hash(bytes);
    // `to_hex` yields the full 64-hex-char digest; truncate to our width.
    let full = digest.to_hex();
    full[..ASSET_HASH_HEX_LEN].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_same_input_same_hash() {
        let a = asset_hash_hex(b"hello content-addressed world");
        let b = asset_hash_hex(b"hello content-addressed world");
        assert_eq!(a, b, "same bytes must yield the same hash");
        assert_eq!(a.len(), ASSET_HASH_HEX_LEN);
    }

    #[test]
    fn different_input_different_hash() {
        let a = asset_hash_hex(b"alpha");
        let b = asset_hash_hex(b"beta");
        assert_ne!(a, b, "different bytes should (w.h.p.) yield different hashes");
    }

    #[test]
    fn matches_known_blake3_prefix() {
        // blake3("") full digest starts af1349b9f5f9a1a6... ; we keep 16 chars.
        let empty = asset_hash_hex(b"");
        assert_eq!(empty, "af1349b9f5f9a1a6");
    }

    #[test]
    fn is_lowercase_hex() {
        let h = asset_hash_hex(b"some bytes");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }
}
