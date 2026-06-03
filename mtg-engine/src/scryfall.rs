//! Scryfall CDN image-URL computation (shared core for mtg-722 / task #7).
//!
//! Both the browser client (Prong A: a compact name→(uuid, version) table
//! shipped as a hashed CAS asset) and `mtg download` (Prong B: local /images
//! prepopulation) need to turn a card's Scryfall identity into the IMMUTABLE
//! Scryfall CDN image URL. This module is the ONE place that knows that URL
//! shape, so the client and the downloader can never drift (DRY).
//!
//! ## Why the CDN, not the API
//!
//! `api.scryfall.com/cards/named?...&format=image` is rate-limited, returns
//! `Cache-Control: max-age=172800` (2 days), and 404s on engine token names
//! like "Clue Token". The direct CDN object
//! `cards.scryfall.io/<size>/front/<a>/<b>/<id>.jpg?<version>` is served
//! `Cache-Control: max-age=31556952, immutable` (1 year, cf-cached) with no
//! API hop or rate limit. task #7 migrates all external image loads onto it.
//!
//! ## URL shape (verified live 2026-06-03)
//!
//! ```text
//! https://cards.scryfall.io/<size>/front/<id[0]>/<id[1]>/<id>.jpg?<version>
//! ```
//! where `<id>` is the card's Scryfall UUID, `<id[0]>`/`<id[1]>` are its first
//! two characters (the CDN's fan-out dirs), and `<version>` is the bare digit
//! string Scryfall appends as the `?` query on every `image_uris` entry for
//! cache-busting. Examples confirmed against the live CDN:
//! - Lightning Bolt id `77c6fa74-…` →
//!   `…/small/front/7/7/77c6fa74-….jpg?1706239968`
//! - Clue token   id `c321b9e4-…` (layout=token) →
//!   `…/small/front/c/3/c321b9e4-….jpg?1771590258`
//!
//! The `(id, version)` pair is exactly what the compact client table stores;
//! the size is chosen per render, so a few bytes per card reconstruct every
//! size's immutable URL.
//!
//! This module is dependency-free (no native-only crates), so the SAME URL
//! computation compiles for the native `mtg download` build AND the wasm
//! client — one implementation, no Rust/JS drift (DRY, task #7).

/// Scryfall CDN host (always https).
const SCRYFALL_CDN: &str = "https://cards.scryfall.io";

/// A Scryfall image size segment, as it appears in the CDN path and the
/// `image_uris` keys. Kept here (not reusing the native-only
/// `download::ImageSize`) so the shared core stays wasm-compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdnSize {
    /// 146×204 — battlefield / thumbnail.
    Small,
    /// 488×680 — detail view.
    Normal,
}

impl CdnSize {
    /// The CDN path + `image_uris` key segment ("small" / "normal").
    pub fn segment(self) -> &'static str {
        match self {
            CdnSize::Small => "small",
            CdnSize::Normal => "normal",
        }
    }
}

/// Build the immutable Scryfall CDN image URL for `(scryfall_id, version)` at
/// `size`. `version` is the bare cache-buster digits (see module docs); pass
/// it WITHOUT a leading `?`.
///
/// The CDN fan-out dirs are the first two characters of the UUID. Scryfall
/// UUIDs are always ≥2 chars, but we guard defensively so a malformed id can
/// never panic (it just yields a URL that 404s, which the client cascade
/// already tolerates).
pub fn cdn_image_url(scryfall_id: &str, version: &str, size: CdnSize) -> String {
    let mut chars = scryfall_id.chars();
    let a = chars.next().unwrap_or('0');
    let b = chars.next().unwrap_or('0');
    format!(
        "{SCRYFALL_CDN}/{}/front/{a}/{b}/{scryfall_id}.jpg?{version}",
        size.segment()
    )
}

/// Extract the bare cache-buster `version` from a full Scryfall `image_uris`
/// URL (the digits after the last `?`). Returns `None` if there is no query.
///
/// Used by the table builder to distil `image_uris.small` (or any size) down
/// to the compact `version` token — the rest of the URL is reconstructable
/// from `(id, size)` via [`cdn_image_url`].
pub fn image_version_from_url(image_uri: &str) -> Option<&str> {
    image_uri.rsplit_once('?').map(|(_, v)| v).filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdn_url_matches_live_scryfall_shape() {
        // Verified against the live CDN 2026-06-03 (see module docs).
        assert_eq!(
            cdn_image_url("77c6fa74-5543-42ac-9ead-0e890b188e99", "1706239968", CdnSize::Small),
            "https://cards.scryfall.io/small/front/7/7/77c6fa74-5543-42ac-9ead-0e890b188e99.jpg?1706239968",
        );
        // The Clue TOKEN resolves on the CDN identically (the whole point of
        // task #7 — api.scryfall named?exact=Clue Token 404s, this does not).
        assert_eq!(
            cdn_image_url("c321b9e4-ab7e-4e8a-988f-5463c776d685", "1771590258", CdnSize::Normal),
            "https://cards.scryfall.io/normal/front/c/3/c321b9e4-ab7e-4e8a-988f-5463c776d685.jpg?1771590258",
        );
    }

    #[test]
    fn version_extraction_round_trips() {
        let small = "https://cards.scryfall.io/small/front/7/7/77c6fa74-5543-42ac-9ead-0e890b188e99.jpg?1706239968";
        assert_eq!(image_version_from_url(small), Some("1706239968"));
        // No query → None (defensive).
        assert_eq!(
            image_version_from_url("https://cards.scryfall.io/small/front/7/7/x.jpg"),
            None
        );
        assert_eq!(image_version_from_url("https://cards.scryfall.io/x.jpg?"), None);

        // Round-trip: (id, extracted version) reconstructs the original URL.
        let id = "77c6fa74-5543-42ac-9ead-0e890b188e99";
        let v = image_version_from_url(small).unwrap();
        assert_eq!(cdn_image_url(id, v, CdnSize::Small), small);
    }
}
