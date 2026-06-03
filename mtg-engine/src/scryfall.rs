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

// ---------------------------------------------------------------------------
// "card-lookup" table — encoding D (columnar), mtg-722 / task #7.
// ---------------------------------------------------------------------------
//
// The shipped CAS asset `card-lookup.<blake3>.bin` maps an engine lookup KEY →
// the card's `(uuid, version, dfc)`, from which both the client and `mtg
// download` reconstruct the immutable cards.scryfall.io URL via
// `cdn_image_url`. Encoding D (columnar) was the smallest brotli of the six
// measured (scratch/scryfall-cdn-experiment: ~866 KB brotli / ~1.3 MB raw over
// ~35k entries; columnar compresses best because each column is homogeneous).
//
// SHIPPING: we ship the RAW .bin (NOT pre-brotli'd + a JS decompressor). The
// blake3 CAS hash is over these raw bytes; the ~866 KB wire size comes from
// HTTP transport compression (tower-http CompressionLayer + Cloudflare), which
// the browser transparently inflates before the client parses the ArrayBuffer.
//
// On-disk layout (little-endian; see [`encode_card_lookup`] for the exact
// bytes): MAGIC "SCDT" | format_version(1) | reserved(0) | u32 count N |
// u32 names_len | names blob ('\n'-joined, SORTED, UTF-8) | uuid column
// (N×16 raw) | version column (N×u32, bit31 = DFC flag). Sorted keys let the
// client binary-search (or build a Map once). The MAGIC + format_version make
// the future "N art-variants per name" picker a NEW format_version=2 (additive),
// never an ambiguous reinterpret of v1 bytes.
//
// KEY convention (task #7): real cards key on the exact engine card name.
// TOKENS key on a COMPOSITE of (name, P/T, colors) — Scryfall has many distinct
// tokens sharing a name ("Elemental" spans 1/1…8/8; "Zombie" has white-1/1,
// black-2/2, …), so a name-only key would mis-render e.g. a 7/1 Elemental as a
// 1/1. [`token_lookup_key`] builds that composite; the client builds the
// identical key from the view model's token P/T + colors. The table ALSO indexes
// a bare-name entry per token (a representative oldest art) so a composite miss
// falls back to *some* correct-name art rather than 404 (build-time aliasing;
// see the generator).
//
// CASCADE (task #7, per user 2026-06-03): [local-if-allowed → CDN-from-table →
// gatherer]. api.scryfall is KILLED entirely (no builder, no ref, no fallback).
// Gatherer is RETAINED as the table-MISS safety net (kept rare by the coverage
// aliasing above), so a miss falls through to gatherer rather than no-image.

/// Field separator inside a composite token key. Unit Separator (0x1F): never
/// appears in a card name, and is neither the record separator ('\n') nor the
/// column separator (0x00) of the table, so it is round-trip safe.
pub const KEY_FIELD_SEP: char = '\u{1f}';

/// Build the composite lookup key for a TOKEN: `name␟P␟T␟colors`, where colors
/// is the card's color letters in WUBRG order (empty = colorless). Mirrors the
/// client's key construction so the Rust generator and the JS lookup agree.
///
/// `power`/`toughness` are the token's printed P/T as strings ("0" for none,
/// "*" allowed); `colors_wubrg` is e.g. "R", "WU", or "" for colorless.
pub fn token_lookup_key(name: &str, power: &str, toughness: &str, colors_wubrg: &str) -> String {
    let s = KEY_FIELD_SEP;
    format!("{name}{s}{power}{s}{toughness}{s}{colors_wubrg}")
}

/// Magic bytes at the head of a card-lookup table file ("ScryfallCardDataTable").
pub const CARD_LOOKUP_MAGIC: &[u8; 4] = b"SCDT";
/// Current on-disk format version. A future "N art-variants per name" picker
/// becomes format_version 2 (additive), so a decoder can reject an unknown
/// format as a hard miss rather than misinterpret bytes.
pub const CARD_LOOKUP_FORMAT_VERSION: u8 = 1;
/// Fixed header size: MAGIC(4) + format_version(1) + reserved(1) + count(4) + names_len(4).
const CARD_LOOKUP_HEADER_LEN: usize = 14;
/// Bit 31 of the version column flags a double-faced card (front/back share
/// uuid+version, verified). The version timestamp is < 2^31, so this is free.
const DFC_FLAG_BIT: u32 = 1 << 31;
/// Mask isolating the version timestamp (bits 0..=30).
const VERSION_MASK: u32 = 0x7FFF_FFFF;

/// One row of the card-lookup table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardArtEntry {
    /// Lookup key — a card name, or a [`token_lookup_key`] composite.
    pub key: String,
    /// The chosen printing's Scryfall UUID (raw 16 bytes).
    pub uuid: [u8; 16],
    /// The image cache-buster `version` (the `?<digits>` from `image_uris`).
    /// Bits 0..=30 only; bit 31 is reserved for the [`CardArtEntry::dfc`] flag
    /// in the encoded form.
    pub version: u32,
    /// True if this card is double-faced (has a back face). The CDN URL for
    /// the back substitutes `front`→`back` in the path, reusing uuid+version.
    pub dfc: bool,
}

/// Parse a Scryfall UUID string ("xxxxxxxx-xxxx-…") into raw 16 bytes.
/// Returns `None` if it is not 32 hex digits (after removing dashes).
pub fn uuid_to_bytes(uuid: &str) -> Option<[u8; 16]> {
    let hex: String = uuid.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Render raw 16 UUID bytes back to the canonical 8-4-4-4-12 hex string.
pub fn uuid_to_string(bytes: &[u8; 16]) -> String {
    let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Encode the card-lookup table in encoding D (UNCOMPRESSED raw bytes — the
/// shipped CAS asset is these raw bytes; HTTP transport compression handles
/// the wire size, so we do NOT pre-brotli + ship a JS decompressor).
///
/// Layout (little-endian):
/// ```text
/// "SCDT" | u8 format_version=1 | u8 reserved=0 | u32 count N | u32 names_len
///        | names blob (N keys '\n'-joined, SORTED ascending, UTF-8)
///        | uuid column  (N × 16 raw bytes)
///        | version column (N × u32; bit31 = DFC flag, bits0..30 = version)
/// ```
/// `entries` MUST be sorted ascending by `key` (the client binary-searches the
/// names column); asserted in debug builds.
pub fn encode_card_lookup(entries: &[CardArtEntry]) -> Vec<u8> {
    debug_assert!(
        entries.windows(2).all(|w| w[0].key <= w[1].key),
        "card-lookup entries must be sorted by key",
    );
    let n = entries.len() as u32;
    let names = entries.iter().map(|e| e.key.as_str()).collect::<Vec<_>>().join("\n");
    let names_bytes = names.as_bytes();
    let mut out = Vec::with_capacity(CARD_LOOKUP_HEADER_LEN + names_bytes.len() + entries.len() * 20);
    out.extend_from_slice(CARD_LOOKUP_MAGIC);
    out.push(CARD_LOOKUP_FORMAT_VERSION);
    out.push(0); // reserved
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&(names_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(names_bytes);
    for e in entries {
        out.extend_from_slice(&e.uuid);
    }
    for e in entries {
        let packed = (e.version & VERSION_MASK) | if e.dfc { DFC_FLAG_BIT } else { 0 };
        out.extend_from_slice(&packed.to_le_bytes());
    }
    out
}

/// Decode encoding D back into entries (for `mtg download` + tests; the browser
/// has its own JS decoder of the SAME layout). Returns `None` on a bad magic,
/// an unknown format_version, or any structural inconsistency — so a format
/// mismatch is a hard miss, never a silent panic or misinterpretation.
pub fn decode_card_lookup(bytes: &[u8]) -> Option<Vec<CardArtEntry>> {
    if bytes.get(0..4)? != CARD_LOOKUP_MAGIC {
        return None;
    }
    if *bytes.get(4)? != CARD_LOOKUP_FORMAT_VERSION {
        return None; // unknown format → hard miss (forward-compat guard)
    }
    // bytes[5] = reserved, ignored.
    let count = u32::from_le_bytes(bytes.get(6..10)?.try_into().ok()?) as usize;
    let names_len = u32::from_le_bytes(bytes.get(10..14)?.try_into().ok()?) as usize;
    let names_blob = bytes.get(CARD_LOOKUP_HEADER_LEN..CARD_LOOKUP_HEADER_LEN + names_len)?;
    let names_str = std::str::from_utf8(names_blob).ok()?;
    let names: Vec<&str> = if names_str.is_empty() {
        Vec::new()
    } else {
        names_str.split('\n').collect()
    };
    if names.len() != count {
        return None;
    }
    let cols = bytes.get(CARD_LOOKUP_HEADER_LEN + names_len..)?;
    let uuid_bytes = count.checked_mul(16)?;
    let ver_bytes = count.checked_mul(4)?;
    if cols.len() != uuid_bytes + ver_bytes {
        return None;
    }
    let (uuid_col, ver_col) = cols.split_at(uuid_bytes);
    let mut out = Vec::with_capacity(count);
    for (i, key) in names.into_iter().enumerate() {
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&uuid_col[i * 16..i * 16 + 16]);
        let packed = u32::from_le_bytes(ver_col[i * 4..i * 4 + 4].try_into().ok()?);
        out.push(CardArtEntry {
            key: key.to_string(),
            uuid,
            version: packed & VERSION_MASK,
            dfc: packed & DFC_FLAG_BIT != 0,
        });
    }
    Some(out)
}

/// A decoded card-lookup table ready for O(1) name/identity → CDN-URL lookups.
///
/// This is the SINGLE source of truth for the client image cascade's CDN rung:
/// the wasm binding wraps one of these in a thread-local and the JS cascade
/// calls [`CardLookupTable::cdn_url`] — so decode, key normalization, and URL
/// construction all live in ONE Rust impl (no JS reimplementation, no drift;
/// task #7 steer #1). Native code (`mtg download`) uses the same type.
#[derive(Debug, Clone, Default)]
pub struct CardLookupTable {
    /// key → (uuid bytes, version, dfc).
    map: std::collections::HashMap<String, ([u8; 16], u32, bool)>,
}

impl CardLookupTable {
    /// Decode an encoding-D (SCDT) blob into a lookup table. `None` on a bad
    /// magic / unknown format / structural error (caller treats as a miss).
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let entries = decode_card_lookup(bytes)?;
        let map = entries
            .into_iter()
            .map(|e| (e.key, (e.uuid, e.version, e.dfc)))
            .collect();
        Some(Self { map })
    }

    /// Number of indexed keys (incl. token composites + aliases).
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Resolve a card to its immutable cards.scryfall.io URL at `size`, or
    /// `None` on a table miss (the cascade then falls through to gatherer).
    ///
    /// Real cards look up by `name`. TOKENS try the composite
    /// (name + P/T + colors) FIRST, then fall back to the bare `name` (a
    /// representative oldest art) so a composite miss still yields correct-name
    /// art rather than nothing. `power`/`toughness` are strings ("" if none,
    /// "*" for variable); `colors_wubrg` is the sorted-WUBRG color string —
    /// the caller MUST normalize identically to the table builder.
    pub fn cdn_url(
        &self,
        name: &str,
        power: &str,
        toughness: &str,
        colors_wubrg: &str,
        is_token: bool,
        size: CdnSize,
    ) -> Option<String> {
        let lookup = |key: &str| {
            self.map
                .get(key)
                .map(|(uuid, ver, _dfc)| cdn_image_url(&uuid_to_string(uuid), &ver.to_string(), size))
        };
        if is_token {
            let composite = token_lookup_key(name, power, toughness, colors_wubrg);
            lookup(&composite).or_else(|| lookup(name))
        } else {
            lookup(name)
        }
    }
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

    #[test]
    fn uuid_bytes_round_trip() {
        let id = "c321b9e4-ab7e-4e8a-988f-5463c776d685";
        let bytes = uuid_to_bytes(id).unwrap();
        assert_eq!(bytes[0], 0xc3);
        assert_eq!(uuid_to_string(&bytes), id);
        // Malformed ids reject (hard miss, no panic).
        assert_eq!(uuid_to_bytes("not-a-uuid"), None);
        assert_eq!(uuid_to_bytes(""), None);
    }

    #[test]
    fn token_key_is_composite_and_collision_free() {
        // Two distinct "Elemental" tokens (7/1 red vs 1/1 colorless) → distinct
        // keys, so a name-only table can't mis-render one as the other.
        let a = token_lookup_key("Elemental", "7", "1", "R");
        let b = token_lookup_key("Elemental", "1", "1", "");
        assert_ne!(a, b);
        assert_eq!(a, "Elemental\u{1f}7\u{1f}1\u{1f}R");
    }

    #[test]
    fn card_lookup_encode_decode_round_trips() {
        // Mixed table: a normal card (name key) + a token (composite key),
        // SORTED ascending by key. Uses the verified Clue token uuid/version.
        let clue_uuid = uuid_to_bytes("c321b9e4-ab7e-4e8a-988f-5463c776d685").unwrap();
        let bolt_uuid = uuid_to_bytes("77c6fa74-5543-42ac-9ead-0e890b188e99").unwrap();
        let mut entries = vec![
            CardArtEntry {
                key: "Clue Token\u{1f}0\u{1f}0\u{1f}".to_string(),
                uuid: clue_uuid,
                version: 1771590258,
                dfc: false,
            },
            // A double-faced card exercises the bit31 DFC flag round-trip.
            CardArtEntry {
                key: "Lightning Bolt".to_string(),
                uuid: bolt_uuid,
                version: 1706239968,
                dfc: true,
            },
        ];
        entries.sort_by(|a, b| a.key.cmp(&b.key));

        let blob = encode_card_lookup(&entries);
        // Official header: MAGIC, format_version, then count at bytes 6..10.
        assert_eq!(&blob[0..4], CARD_LOOKUP_MAGIC);
        assert_eq!(blob[4], CARD_LOOKUP_FORMAT_VERSION);
        assert_eq!(u32::from_le_bytes(blob[6..10].try_into().unwrap()), 2);

        let decoded = decode_card_lookup(&blob).expect("decodes");
        assert_eq!(decoded, entries);
        // DFC flag survives (and never leaks into the version int).
        let bolt = decoded.iter().find(|e| e.key == "Lightning Bolt").unwrap();
        assert!(bolt.dfc);
        assert_eq!(bolt.version, 1706239968);

        // End-to-end: a decoded token entry reconstructs its live CDN URL.
        let clue = decoded.iter().find(|e| e.key.starts_with("Clue Token")).unwrap();
        assert_eq!(
            cdn_image_url(&uuid_to_string(&clue.uuid), &clue.version.to_string(), CdnSize::Normal),
            "https://cards.scryfall.io/normal/front/c/3/c321b9e4-ab7e-4e8a-988f-5463c776d685.jpg?1771590258",
        );

        // Empty table round-trips (N=0, empty names blob).
        assert_eq!(decode_card_lookup(&encode_card_lookup(&[])).unwrap(), Vec::new());
        // Bad magic / unknown format / truncation → None (hard miss, never panic).
        assert_eq!(
            decode_card_lookup(b"XXXX\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00"),
            None
        );
        let mut wrong_fmt = blob.clone();
        wrong_fmt[4] = 99;
        assert_eq!(decode_card_lookup(&wrong_fmt), None);
        assert_eq!(decode_card_lookup(&[1, 2]), None);
        assert_eq!(decode_card_lookup(&blob[..blob.len() - 3]), None);
    }

    #[test]
    fn card_lookup_table_resolves_name_token_and_fallback() {
        let clue = uuid_to_bytes("c321b9e4-ab7e-4e8a-988f-5463c776d685").unwrap();
        let bolt = uuid_to_bytes("77c6fa74-5543-42ac-9ead-0e890b188e99").unwrap();
        let goblin = uuid_to_bytes("11111111-2222-3333-4444-555555555555").unwrap();
        // Table: normal card by name; Clue token by composite + bare-name alias;
        // a "Goblin" bare name only (composite will miss → falls back to it).
        let mut entries = vec![
            CardArtEntry {
                key: "Lightning Bolt".into(),
                uuid: bolt,
                version: 1706239968,
                dfc: false,
            },
            CardArtEntry {
                key: token_lookup_key("Clue", "", "", ""),
                uuid: clue,
                version: 1771590258,
                dfc: false,
            },
            CardArtEntry {
                key: "Clue".into(),
                uuid: clue,
                version: 1771590258,
                dfc: false,
            },
            CardArtEntry {
                key: "Goblin".into(),
                uuid: goblin,
                version: 42,
                dfc: false,
            },
        ];
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        let table = CardLookupTable::from_bytes(&encode_card_lookup(&entries)).unwrap();

        // Normal card by name.
        assert_eq!(
            table.cdn_url("Lightning Bolt", "", "", "", false, CdnSize::Small),
            Some(
                "https://cards.scryfall.io/small/front/7/7/77c6fa74-5543-42ac-9ead-0e890b188e99.jpg?1706239968".into()
            ),
        );
        // Token by composite key (exact identity).
        assert_eq!(
            table.cdn_url("Clue", "", "", "", true, CdnSize::Normal),
            Some(
                "https://cards.scryfall.io/normal/front/c/3/c321b9e4-ab7e-4e8a-988f-5463c776d685.jpg?1771590258".into()
            ),
        );
        // Token whose composite MISSES (Goblin 7/1 red not indexed) → bare-name fallback.
        assert!(table
            .cdn_url("Goblin", "7", "1", "R", true, CdnSize::Small)
            .unwrap()
            .contains("11111111"));
        // Genuine miss → None (cascade falls through to gatherer).
        assert_eq!(
            table.cdn_url("Nonexistent Card", "", "", "", false, CdnSize::Small),
            None
        );
    }
}
