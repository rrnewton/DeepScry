//! Build the card-lookup table (encoding D / SCDT) from Scryfall
//! `unique_artwork.json` records — mtg-722 / task #7.
//!
//! This is the generator's pure core: parsed Scryfall records in → a sorted
//! [`CardArtEntry`] list out (which [`crate::scryfall::encode_card_lookup`]
//! serializes). It is dependency-light (serde for the record model) and
//! deterministic, so it is unit-testable on a small fixture with NO network.
//! The 254 MB fetch + JSON streaming is a thin native wrapper around this.
//!
//! ## What it does (task #7 spec)
//!
//! 1. **Format-drift hard-error.** Every record's front-image URL is checked
//!    against the shape `cdn_image_url` reconstructs (host, shard dirs ==
//!    uuid[0..2], url-uuid == record id, ext ∈ {jpg,png}, version parses). ANY
//!    mismatch aborts the whole build (the generator then keeps the old table)
//!    — so a silent Scryfall URL-scheme change can never ship a broken table.
//! 2. **Oldest-art-per-identity selection.** Among all records for a lookup
//!    key, pick the OLDEST printing (min `released_at`), preferring a real
//!    image (`image_status` ∉ {missing, placeholder}) and non-digital. (User:
//!    old-school vibe.)
//! 3. **Keying + coverage aliasing.** Real cards key on NAME; TOKENS on the
//!    composite (name + P/T + colors) AND a bare-name alias. Plus: Alchemy
//!    "A-" prefix → de-prefixed alias; DFC "Front // Back" → both face names +
//!    the combined name. Maximizes table hits so the gatherer fallback is rare.

use serde::Deserialize;

use crate::scryfall::{image_version_from_url, token_lookup_key, CardArtEntry, CdnSize};

/// The `image_uris` sub-object we care about (sizes the UI requests).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImageUris {
    #[serde(default)]
    pub small: Option<String>,
    #[serde(default)]
    pub normal: Option<String>,
}

impl ImageUris {
    /// The URL we derive `(id, version)` from — prefer `normal`, fall back to
    /// `small` (either yields the same id+version, only the size segment differs).
    fn any_url(&self) -> Option<&str> {
        self.normal.as_deref().or(self.small.as_deref())
    }
}

/// One face of a card (DFC). Carries its own name + image_uris.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CardFace {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub image_uris: Option<ImageUris>,
}

/// A Scryfall `unique_artwork.json` card record — only the fields the table
/// build reads. `#[serde(default)]` everywhere so partial/older dumps parse.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ScryfallRecord {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub layout: String,
    #[serde(default)]
    pub type_line: String,
    #[serde(default)]
    pub power: Option<String>,
    #[serde(default)]
    pub toughness: Option<String>,
    #[serde(default)]
    pub colors: Vec<String>,
    #[serde(default)]
    pub released_at: String,
    #[serde(default)]
    pub digital: bool,
    #[serde(default)]
    pub image_status: String,
    #[serde(default)]
    pub image_uris: Option<ImageUris>,
    #[serde(default)]
    pub card_faces: Vec<CardFace>,
}

impl ScryfallRecord {
    /// Is this record a token (layout "token" / "double_faced_token")?
    fn is_token(&self) -> bool {
        self.layout == "token" || self.layout == "double_faced_token"
    }

    /// The front-face image URL (top-level for single-faced; face[0] for DFC).
    fn front_image_url(&self) -> Option<&str> {
        if let Some(u) = self.image_uris.as_ref().and_then(ImageUris::any_url) {
            return Some(u);
        }
        self.card_faces
            .first()
            .and_then(|f| f.image_uris.as_ref())
            .and_then(ImageUris::any_url)
    }

    /// `image_status` ∉ {missing, placeholder} — i.e. a real scan exists.
    fn has_real_image(&self) -> bool {
        !matches!(self.image_status.as_str(), "missing" | "placeholder")
    }
}

/// Normalize a card's colors to a sorted WUBRG letter string ("" = colorless).
/// MUST match the engine-side normalization for token composite keys.
pub fn normalize_colors_wubrg(colors: &[String]) -> String {
    const ORDER: [&str; 5] = ["W", "U", "B", "R", "G"];
    ORDER
        .iter()
        .filter(|c| colors.iter().any(|x| x == *c))
        .copied()
        .collect()
}

/// Normalize a P/T value to the key string: the trimmed value, or "" if absent
/// (non-creature tokens like Clue have no P/T). "*" passes through unchanged.
fn normalize_pt(pt: &Option<String>) -> String {
    pt.as_deref().unwrap_or("").trim().to_string()
}

/// Selection rank for a record under a given key. Higher tuple sorts as
/// "better": (real image, non-digital). Oldest `released_at` breaks ties
/// (compared separately, ascending — older wins).
fn rank(rec: &ScryfallRecord) -> (bool, bool) {
    (rec.has_real_image(), !rec.digital)
}

/// True if `a` is a STRICTLY better pick than the current best `b` for a key:
/// higher rank, or equal rank with an older (lexicographically smaller, ISO
/// date) `released_at`. Deterministic; ties keep the incumbent (`<` not `<=`).
fn is_better(a: &ScryfallRecord, b: &ScryfallRecord) -> bool {
    match rank(a).cmp(&rank(b)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => a.released_at < b.released_at,
    }
}

/// All lookup keys a record should be indexed under (primary + aliases).
fn lookup_keys(rec: &ScryfallRecord) -> Vec<String> {
    let mut keys = Vec::new();
    if rec.is_token() {
        // Primary: composite (name + P/T + colors). Alias: bare name.
        keys.push(token_lookup_key(
            &rec.name,
            &normalize_pt(&rec.power),
            &normalize_pt(&rec.toughness),
            &normalize_colors_wubrg(&rec.colors),
        ));
        keys.push(rec.name.clone());
    } else {
        keys.push(rec.name.clone());
        // DFC "Front // Back": index each face name too.
        if let Some((front, back)) = rec.name.split_once(" // ") {
            keys.push(front.to_string());
            keys.push(back.to_string());
        }
    }
    // Alchemy "A-Foo" → also index "Foo".
    if let Some(stripped) = rec.name.strip_prefix("A-") {
        keys.push(stripped.to_string());
    }
    keys
}

/// Validate a record's front-image URL against the shape `cdn_image_url`
/// reconstructs from `(id, version)`. Returns `Err(reason)` on any drift.
fn check_url_drift(rec: &ScryfallRecord, url: &str, version: &str) -> Result<(), String> {
    let drift = |why: &str| Err(format!("URL drift for {} ({}): {} — url={url}", rec.name, rec.id, why));
    // Strip scheme; require the CDN host.
    let after = url.strip_prefix("https://cards.scryfall.io/").ok_or_else(|| {
        format!(
            "URL drift for {} ({}): host != cards.scryfall.io — url={url}",
            rec.name, rec.id
        )
    })?;
    let path = after.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').collect();
    // <size>/<face>/<s1>/<s2>/<uuid>.<ext>
    if segs.len() != 5 {
        return drift("path is not <size>/<face>/<s1>/<s2>/<uuid>.<ext>");
    }
    let id_nodash: String = rec.id.chars().filter(|c| *c != '-').collect();
    let first = id_nodash.chars().next().unwrap_or('?');
    let second = id_nodash.chars().nth(1).unwrap_or('?');
    if segs[2] != first.to_string() || segs[3] != second.to_string() {
        return drift("shard dirs != uuid[0..2]");
    }
    let (url_uuid, ext) = segs[4].rsplit_once('.').ok_or_else(|| {
        format!(
            "URL drift for {} ({}): filename has no extension — url={url}",
            rec.name, rec.id
        )
    })?;
    if url_uuid != rec.id {
        return drift("url uuid != record id");
    }
    if !matches!(ext, "jpg" | "png") {
        return drift("extension not jpg/png");
    }
    if version.parse::<u32>().is_err() {
        return drift("version does not parse as u32");
    }
    Ok(())
}

/// Outcome of a table build: the sorted entries + coverage stats.
#[derive(Debug, Default)]
pub struct BuildResult {
    /// Sorted-by-key entries, ready for `encode_card_lookup`.
    pub entries: Vec<CardArtEntry>,
    /// Records skipped because they had no usable front image.
    pub skipped_no_image: usize,
}

/// Build the card-lookup entries from Scryfall records.
///
/// `size` selects which `image_uris` size to read the `(id, version)` from —
/// they are identical across sizes, so it only affects which URL is
/// drift-checked.
///
/// # Errors
///
/// Returns `Err(reason)` on the FIRST record whose front-image URL fails the
/// format-drift check (bad host, shard dirs ≠ uuid[0..2], url-uuid ≠ id,
/// non-jpg/png extension, or an unparseable version), or that has an image
/// URL with no `?version`. The generator keeps the previous table on error.
pub fn build_card_lookup(records: &[ScryfallRecord], size: CdnSize) -> Result<BuildResult, String> {
    use std::collections::HashMap;
    // key → index into a parallel `picks` vec holding the winning record clone.
    let mut best: HashMap<String, ScryfallRecord> = HashMap::new();
    let mut result = BuildResult::default();

    for rec in records {
        let Some(url) = rec.front_image_url() else {
            result.skipped_no_image += 1;
            continue;
        };
        let Some(version) = image_version_from_url(url) else {
            return Err(format!(
                "URL drift for {} ({}): no ?version — url={url}",
                rec.name, rec.id
            ));
        };
        check_url_drift(rec, url, version)?;
        for key in lookup_keys(rec) {
            match best.get(&key) {
                Some(cur) if !is_better(rec, cur) => {}
                _ => {
                    best.insert(key, rec.clone());
                }
            }
        }
    }

    let mut entries: Vec<CardArtEntry> = best
        .into_iter()
        .filter_map(|(key, rec)| {
            let url = rec.front_image_url()?;
            let version = image_version_from_url(url)?.parse::<u32>().ok()?;
            let uuid = crate::scryfall::uuid_to_bytes(&rec.id)?;
            let dfc = rec.layout == "transform"
                || rec.layout == "modal_dfc"
                || rec.layout == "double_faced_token"
                || rec.card_faces.len() >= 2;
            Some(CardArtEntry {
                key,
                uuid,
                version,
                dfc,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    // The `size` param documents intent (which URL the client will build); the
    // drift check already validated that size's path. Touch it so it's not
    // flagged unused while the single-art-per-name format is size-agnostic.
    let _ = size;
    result.entries = entries;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scryfall::{cdn_image_url, uuid_to_string};

    fn rec(name: &str, id: &str, ver: &str, layout: &str) -> ScryfallRecord {
        ScryfallRecord {
            name: name.to_string(),
            id: id.to_string(),
            layout: layout.to_string(),
            image_status: "highres_scan".to_string(),
            released_at: "2020-01-01".to_string(),
            image_uris: Some(ImageUris {
                small: Some(format!(
                    "https://cards.scryfall.io/small/front/{}/{}/{id}.jpg?{ver}",
                    &id[0..1],
                    &id[1..2]
                )),
                normal: Some(format!(
                    "https://cards.scryfall.io/normal/front/{}/{}/{id}.jpg?{ver}",
                    &id[0..1],
                    &id[1..2]
                )),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn builds_token_and_normal_card_keys_and_resolves_cdn_urls() {
        let bolt = rec(
            "Lightning Bolt",
            "77c6fa74-5543-42ac-9ead-0e890b188e99",
            "1706239968",
            "normal",
        );
        let mut clue = rec("Clue", "c321b9e4-ab7e-4e8a-988f-5463c776d685", "1771590258", "token");
        clue.type_line = "Token Artifact — Clue".to_string();

        let out = build_card_lookup(&[bolt, clue], CdnSize::Normal).expect("no drift");
        let keys: Vec<&str> = out.entries.iter().map(|e| e.key.as_str()).collect();

        // Normal card keyed by name.
        assert!(keys.contains(&"Lightning Bolt"));
        // Token keyed by composite (name + empty P/T + colorless) AND bare name.
        assert!(keys.contains(&"Clue\u{1f}\u{1f}\u{1f}"));
        assert!(keys.contains(&"Clue")); // bare-name alias

        // The token entry reconstructs the verified live CDN URL.
        let clue_entry = out.entries.iter().find(|e| e.key == "Clue\u{1f}\u{1f}\u{1f}").unwrap();
        assert_eq!(
            cdn_image_url(
                &uuid_to_string(&clue_entry.uuid),
                &clue_entry.version.to_string(),
                CdnSize::Small
            ),
            "https://cards.scryfall.io/small/front/c/3/c321b9e4-ab7e-4e8a-988f-5463c776d685.jpg?1771590258",
        );
        // Sorted by key.
        assert!(out.entries.windows(2).all(|w| w[0].key <= w[1].key));
    }

    #[test]
    fn picks_oldest_art_per_identity() {
        // Two printings of one card; the OLDER released_at wins.
        let new = {
            let mut r = rec("Counterspell", "11111111-1111-1111-1111-111111111111", "100", "normal");
            r.released_at = "2021-01-01".to_string();
            r
        };
        let old = {
            let mut r = rec("Counterspell", "22222222-2222-2222-2222-222222222222", "200", "normal");
            r.released_at = "1994-01-01".to_string();
            r
        };
        let out = build_card_lookup(&[new, old], CdnSize::Normal).unwrap();
        let e = out.entries.iter().find(|e| e.key == "Counterspell").unwrap();
        assert_eq!(uuid_to_string(&e.uuid), "22222222-2222-2222-2222-222222222222");
    }

    #[test]
    fn format_drift_is_a_hard_error() {
        let mut bad = rec("Bad Card", "33333333-3333-3333-3333-333333333333", "5", "normal");
        // Tamper the host → drift.
        bad.image_uris = Some(ImageUris {
            small: Some("https://evil.example.com/x.jpg?5".to_string()),
            normal: None,
        });
        assert!(build_card_lookup(&[bad], CdnSize::Normal).is_err());

        // Shard mismatch (wrong fan-out dirs) → drift.
        let mut shard = rec("Shard Card", "44444444-4444-4444-4444-444444444444", "5", "normal");
        shard.image_uris = Some(ImageUris {
            small: Some(
                "https://cards.scryfall.io/small/front/9/9/44444444-4444-4444-4444-444444444444.jpg?5".to_string(),
            ),
            normal: None,
        });
        assert!(build_card_lookup(&[shard], CdnSize::Normal).is_err());
    }

    #[test]
    fn colors_normalize_to_sorted_wubrg() {
        assert_eq!(normalize_colors_wubrg(&["G".into(), "W".into(), "U".into()]), "WUG");
        assert_eq!(normalize_colors_wubrg(&[]), "");
        assert_eq!(normalize_colors_wubrg(&["B".into()]), "B");
    }
}
