//! Cloudflare R2 deck storage: AWS SigV4 *presigned URL* generation and a
//! swappable [`Identity`] seam for per-user prefix scoping (mtg-742).
//!
//! ## Why presigned URLs (and not a deck-bytes proxy)
//!
//! The converged design (mtg-742) makes R2 the **store of record** for a
//! user's deck collection, and the Rust web server is explicitly NOT in the
//! deck-bytes path. Instead the server holds ONE long-lived "parent" R2 API
//! token (from the environment) and, on request, mints a **short-TTL,
//! prefix-scoped presigned URL** that the browser uses to PUT/GET/HEAD its
//! own collection object directly against R2. Bytes never transit our box.
//!
//! A presigned URL is just an ordinary S3 SigV4 request with the signature
//! carried in the query string instead of an `Authorization` header. We
//! compute it locally from the parent token — **no R2 round-trip** — so the
//! mint endpoint is cheap and offline-testable.
//!
//! ## Prefix scoping & the [`Identity`] seam
//!
//! Each user's collection lives under `decks/<identity>/collection.tgz`.
//! The OAuth login leg (blocked on the user provisioning the OAuth app) is
//! NOT built here; instead [`Identity`] is a trait with a stub
//! [`DevIdentity`] implementation that returns a FIXED `dev` prefix. When
//! OAuth lands, a real `OAuthIdentity` drops in WITHOUT reworking the
//! storage path: the endpoint resolves an `Identity`, asks it for the
//! caller's stable id, and presigns only that caller's key.
//!
//! IMPORTANT: a presigned URL is scoped to ONE exact object key + method.
//! It is NOT a "list everything under my prefix" capability. We presign the
//! caller's single collection key, so even though the parent token can see
//! the whole bucket, the URL we hand out can only touch that one object.
//!
//! ## SigV4 implementation note
//!
//! We use `ring` (already pulled in by rustls via the `web-server` feature)
//! for HMAC-SHA256 and SHA-256, and `percent-encoding` for the strict
//! RFC-3986 query encoding S3 requires. No new crypto dependency, no
//! `aws-sdk-*` behemoth.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};
use ring::{digest, hmac};

/// R2 region is always the literal `auto` (Cloudflare R2 ignores region but
/// SigV4 requires *a* region in the credential scope; their docs mandate
/// `auto`).
const R2_REGION: &str = "auto";
/// SigV4 service name for the S3-compatible API.
const S3_SERVICE: &str = "s3";
/// SigV4 algorithm identifier.
const ALGORITHM: &str = "AWS4-HMAC-SHA256";
/// `x-amz-content-sha256` value used for presigned URLs (the body is not
/// signed; the browser sends arbitrary bytes).
const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

/// Default presigned-URL lifetime: 10 minutes (within the 5–15 min window
/// the design specifies). Long enough for a hydrate+edit+save round-trip,
/// short enough that a leaked URL expires quickly.
pub const DEFAULT_PRESIGN_TTL: Duration = Duration::from_secs(10 * 60);

/// The object key (relative to a user's prefix) holding their whole deck
/// collection as one gzipped tar. ONE object per user keeps writes atomic
/// and makes the If-Match conditional-write story simple.
pub const COLLECTION_OBJECT: &str = "collection.tgz";

/// RFC-3986 "unreserved" set for S3 path/query encoding: everything that is
/// NOT alphanumeric or one of `-_.~` gets percent-encoded. Note that the
/// forward slash separating path segments is encoded per-segment by the
/// caller, NOT here.
const SIGV4_ENCODE: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

/// Same as [`SIGV4_ENCODE`] but ALSO leaves `/` unescaped — used for the
/// canonical URI path, where slashes are real segment separators.
const SIGV4_ENCODE_PATH: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~')
    .remove(b'/');

/// The HTTP method a presigned URL authorizes. A presigned URL is bound to
/// exactly one method; we only ever need these three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresignMethod {
    /// Upload (conditional write happens via the browser's `If-Match` header,
    /// which is NOT part of the signature — R2 honours it regardless).
    Put,
    /// Download / hydrate.
    Get,
    /// Existence + ETag probe.
    Head,
}

impl PresignMethod {
    fn as_str(self) -> &'static str {
        match self {
            PresignMethod::Put => "PUT",
            PresignMethod::Get => "GET",
            PresignMethod::Head => "HEAD",
        }
    }
}

/// Identity of the caller, used to scope storage to `decks/<id>/...`.
///
/// This is the swappable seam for the OAuth leg. The `dev` stub returns a
/// fixed id so the storage path can be built and tested end-to-end before
/// login exists; a real OAuth-backed implementation slots in later without
/// touching any presigning code.
pub trait Identity: Send + Sync {
    /// A stable, filesystem/URL-safe identifier for this caller. MUST match
    /// `[a-z0-9_-]+` so it composes cleanly into an object key prefix and
    /// cannot escape its prefix (no `/`, no `..`).
    fn user_id(&self) -> &str;
}

/// Stub identity for the OAuth-independent storage leg: every caller maps to
/// the single shared `dev` prefix (`decks/dev/`). Replace with an
/// OAuth-backed identity once the login app is provisioned (mtg-742).
#[derive(Debug, Clone, Default)]
pub struct DevIdentity;

impl Identity for DevIdentity {
    fn user_id(&self) -> &str {
        "dev"
    }
}

/// The parent R2 credentials + bucket coordinates, read from the
/// environment at server start. The server holds exactly one of these and
/// NEVER hands the raw secret to a client — only short-lived presigned URLs
/// derived from it.
#[derive(Clone)]
pub struct R2Config {
    /// `AWS_ACCESS_KEY_ID`.
    pub access_key_id: String,
    /// `AWS_SECRET_ACCESS_KEY` (never logged, never serialized to clients).
    pub secret_access_key: String,
    /// `R2_ENDPOINT`, e.g. `https://<acct>.r2.cloudflarestorage.com`.
    /// Stored WITHOUT a trailing slash and WITHOUT the bucket.
    pub endpoint: String,
    /// `R2_BUCKET`, e.g. `deepscry-decks`.
    pub bucket: String,
}

impl std::fmt::Debug for R2Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately redact the secret; only structural fields are shown.
        f.debug_struct("R2Config")
            .field("access_key_id", &"<redacted>")
            .field("secret_access_key", &"<redacted>")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .finish()
    }
}

impl R2Config {
    /// Read R2 config from the standard env vars (mirrors how `.r2.env` is
    /// `source`d, and how `.deepscry-deploy.env` feeds the systemd unit).
    /// Returns `None` if any required var is missing/empty — the deck
    /// storage endpoint is then simply disabled (404), which is the correct
    /// behaviour for a dev box that hasn't been given R2 creds.
    pub fn from_env() -> Option<Self> {
        let access_key_id = non_empty_env("AWS_ACCESS_KEY_ID")?;
        let secret_access_key = non_empty_env("AWS_SECRET_ACCESS_KEY")?;
        let endpoint = non_empty_env("R2_ENDPOINT")?;
        let bucket = non_empty_env("R2_BUCKET")?;
        Some(Self {
            access_key_id,
            secret_access_key,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            bucket,
        })
    }

    /// The bucket host (no scheme), e.g.
    /// `<acct>.r2.cloudflarestorage.com`. Used as the SigV4 `host` header.
    fn host(&self) -> &str {
        self.endpoint
            .strip_prefix("https://")
            .or_else(|| self.endpoint.strip_prefix("http://"))
            .unwrap_or(&self.endpoint)
    }

    /// Build the per-user collection object key: `decks/<id>/collection.tgz`.
    /// The id is validated by [`is_valid_user_id`] before this is called.
    pub fn collection_key(user_id: &str) -> String {
        format!("decks/{user_id}/{COLLECTION_OBJECT}")
    }

    /// Generate a presigned URL for `method` on `object_key`, valid for
    /// `ttl`. The returned URL embeds the signature in its query string and
    /// can be used directly by the browser (fetch / XHR).
    ///
    /// `now` is injected for deterministic testing; production callers pass
    /// `SystemTime::now()`.
    pub fn presign(&self, method: PresignMethod, object_key: &str, ttl: Duration, now: SystemTime) -> String {
        let (amz_date, datestamp) = format_amz_times(now);
        let host = self.host();

        // Canonical URI: /<bucket>/<key>, each segment percent-encoded but
        // slashes preserved (R2 uses path-style addressing).
        let canonical_path = format!("/{}/{}", self.bucket, object_key);
        let canonical_uri = encode_path(&canonical_path);

        let credential_scope = format!("{datestamp}/{R2_REGION}/{S3_SERVICE}/aws4_request");
        let credential = format!("{}/{}", self.access_key_id, credential_scope);

        // Query parameters MUST be sorted by key for the canonical query
        // string. We build them, sort, then join.
        let expires = ttl.as_secs().to_string();
        let mut params: Vec<(String, String)> = vec![
            ("X-Amz-Algorithm".to_string(), ALGORITHM.to_string()),
            ("X-Amz-Credential".to_string(), credential),
            ("X-Amz-Date".to_string(), amz_date.clone()),
            ("X-Amz-Expires".to_string(), expires),
            ("X-Amz-SignedHeaders".to_string(), "host".to_string()),
        ];
        params.sort_by(|a, b| a.0.cmp(&b.0));
        let canonical_query = canonical_query_string(&params);

        // Canonical headers: only `host` is signed for a presigned URL.
        let canonical_headers = format!("host:{host}\n");
        let signed_headers = "host";

        let canonical_request = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            method.as_str(),
            canonical_uri,
            canonical_query,
            canonical_headers,
            signed_headers,
            UNSIGNED_PAYLOAD
        );

        let hashed_canonical_request = hex_sha256(canonical_request.as_bytes());
        let string_to_sign = format!("{ALGORITHM}\n{amz_date}\n{credential_scope}\n{hashed_canonical_request}");

        let signing_key = self.derive_signing_key(&datestamp);
        let signature = hex(hmac_sha256(&signing_key, string_to_sign.as_bytes()).as_ref());

        format!(
            "{}://{}{}?{}&X-Amz-Signature={}",
            scheme(&self.endpoint),
            host,
            canonical_uri,
            canonical_query,
            signature
        )
    }

    /// Presign a GET that forces the browser to download an attachment named
    /// `filename` (the "Download my decks" / data-liberation property). This
    /// is a plain SigV4 GET plus a SIGNED `response-content-disposition`
    /// override query param — S3/R2 echo it back as the response header.
    ///
    /// Because the disposition param is part of the canonical query string it
    /// is covered by the signature, so it cannot be tampered with after the
    /// fact.
    pub fn presign_download(&self, object_key: &str, ttl: Duration, now: SystemTime, filename: &str) -> String {
        let (amz_date, datestamp) = format_amz_times(now);
        let host = self.host();
        let canonical_path = format!("/{}/{}", self.bucket, object_key);
        let canonical_uri = encode_path(&canonical_path);

        let credential_scope = format!("{datestamp}/{R2_REGION}/{S3_SERVICE}/aws4_request");
        let credential = format!("{}/{}", self.access_key_id, credential_scope);
        let expires = ttl.as_secs().to_string();
        let disposition = format!("attachment; filename=\"{filename}\"");

        let mut params: Vec<(String, String)> = vec![
            ("X-Amz-Algorithm".to_string(), ALGORITHM.to_string()),
            ("X-Amz-Credential".to_string(), credential),
            ("X-Amz-Date".to_string(), amz_date.clone()),
            ("X-Amz-Expires".to_string(), expires),
            ("X-Amz-SignedHeaders".to_string(), "host".to_string()),
            ("response-content-disposition".to_string(), disposition),
        ];
        params.sort_by(|a, b| a.0.cmp(&b.0));
        let canonical_query = canonical_query_string(&params);

        let canonical_headers = format!("host:{host}\n");
        let canonical_request =
            format!("GET\n{canonical_uri}\n{canonical_query}\nhost:{host}\n\nhost\n{UNSIGNED_PAYLOAD}",);
        // NB: the explicit `canonical_headers` var keeps the structure obvious
        // even though it's inlined above for the single signed header.
        let _ = canonical_headers;

        let hashed = hex_sha256(canonical_request.as_bytes());
        let string_to_sign = format!("{ALGORITHM}\n{amz_date}\n{credential_scope}\n{hashed}");
        let signing_key = self.derive_signing_key(&datestamp);
        let signature = hex(hmac_sha256(&signing_key, string_to_sign.as_bytes()).as_ref());

        format!(
            "{}://{}{}?{}&X-Amz-Signature={}",
            scheme(&self.endpoint),
            host,
            canonical_uri,
            canonical_query,
            signature
        )
    }

    /// SigV4 four-step signing-key derivation.
    fn derive_signing_key(&self, datestamp: &str) -> Vec<u8> {
        let k_secret = format!("AWS4{}", self.secret_access_key);
        let k_date = hmac_sha256_raw(k_secret.as_bytes(), datestamp.as_bytes());
        let k_region = hmac_sha256_raw(&k_date, R2_REGION.as_bytes());
        let k_service = hmac_sha256_raw(&k_region, S3_SERVICE.as_bytes());
        hmac_sha256_raw(&k_service, b"aws4_request")
    }
}

/// Validate a user id is safe to embed in an object key prefix. Rejects
/// anything that could escape the prefix or break the key grammar. This is
/// the security boundary that keeps a (future, possibly hostile) identity
/// provider from handing us `../` or absolute paths.
pub fn is_valid_user_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

// ─── small helpers (no new deps) ──────────────────────────────────────────

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn scheme(endpoint: &str) -> &str {
    if endpoint.starts_with("http://") {
        "http"
    } else {
        "https"
    }
}

/// Percent-encode a URI path, preserving `/`.
fn encode_path(path: &str) -> String {
    percent_encoding::utf8_percent_encode(path, SIGV4_ENCODE_PATH).to_string()
}

/// Percent-encode a single query value/key (slashes ARE encoded here).
fn encode_query_component(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, SIGV4_ENCODE).to_string()
}

/// Build a canonical query string from already-sorted params.
fn canonical_query_string(sorted_params: &[(String, String)]) -> String {
    sorted_params
        .iter()
        .map(|(k, v)| format!("{}={}", encode_query_component(k), encode_query_component(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> hmac::Tag {
    let k = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::sign(&k, data)
}

fn hmac_sha256_raw(key: &[u8], data: &[u8]) -> Vec<u8> {
    hmac_sha256(key, data).as_ref().to_vec()
}

fn hex_sha256(data: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, data).as_ref())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit(u32::from(b >> 4), 16).unwrap());
        s.push(char::from_digit(u32::from(b & 0xf), 16).unwrap());
    }
    s
}

/// Format a `SystemTime` as the two timestamps SigV4 needs:
/// (`YYYYMMDDTHHMMSSZ`, `YYYYMMDD`). Pure UTC, no leap seconds, no DST —
/// computed from the Unix epoch with a civil-from-days conversion so we
/// don't need the `time`/`chrono` formatting features.
fn format_amz_times(now: SystemTime) -> (String, String) {
    let secs = now.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (year, month, day) = civil_from_days(days);
    let datestamp = format!("{year:04}{month:02}{day:02}");
    let amz_date = format!("{datestamp}T{hour:02}{minute:02}{second:02}Z");
    (amz_date, datestamp)
}

/// Convert a count of days since 1970-01-01 to a (year, month, day) civil
/// date. Algorithm from Howard Hinnant's `civil_from_days` (public domain),
/// valid for the full proleptic Gregorian range we care about.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_known_dates() {
        // Epoch.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // A known leap day.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        // 2026-06-10 is 20614 days after epoch.
        assert_eq!(civil_from_days(20_614), (2026, 6, 10));
    }

    #[test]
    fn amz_times_format() {
        // 2026-06-10T12:34:56Z = 1781440496 epoch secs.
        let t = UNIX_EPOCH + Duration::from_secs(1_781_094_896);
        let (amz, date) = format_amz_times(t);
        assert_eq!(date, "20260610");
        assert_eq!(amz, "20260610T123456Z");
    }

    #[test]
    fn user_id_validation_blocks_traversal() {
        assert!(is_valid_user_id("dev"));
        assert!(is_valid_user_id("github-12345"));
        assert!(is_valid_user_id("a_b-c9"));
        // Anything that could escape the prefix or break the grammar:
        assert!(!is_valid_user_id(""));
        assert!(!is_valid_user_id("../etc"));
        assert!(!is_valid_user_id("a/b"));
        assert!(!is_valid_user_id("UPPER"));
        assert!(!is_valid_user_id("space here"));
        assert!(!is_valid_user_id("dots.bad"));
    }

    #[test]
    fn collection_key_shape() {
        assert_eq!(R2Config::collection_key("dev"), "decks/dev/collection.tgz");
    }

    fn test_config() -> R2Config {
        R2Config {
            access_key_id: "AKIDEXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_string(),
            endpoint: "https://acct.r2.cloudflarestorage.com".to_string(),
            bucket: "deepscry-decks".to_string(),
        }
    }

    /// The presigned URL is well-formed and DETERMINISTIC for a fixed clock
    /// — same inputs always yield the same signature. This both documents
    /// the contract and guards against accidental nondeterminism (e.g. an
    /// unsorted query string).
    #[test]
    fn presign_is_deterministic_and_well_formed() {
        let cfg = test_config();
        let t = UNIX_EPOCH + Duration::from_secs(1_781_094_896);
        let key = R2Config::collection_key("dev");

        let url1 = cfg.presign(PresignMethod::Put, &key, DEFAULT_PRESIGN_TTL, t);
        let url2 = cfg.presign(PresignMethod::Put, &key, DEFAULT_PRESIGN_TTL, t);
        assert_eq!(url1, url2, "presign must be deterministic for a fixed clock");

        // Structural checks on the URL.
        assert!(url1.starts_with("https://acct.r2.cloudflarestorage.com/deepscry-decks/decks/dev/collection.tgz?"));
        assert!(url1.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(url1.contains("X-Amz-Credential=AKIDEXAMPLE%2F20260610%2Fauto%2Fs3%2Faws4_request"));
        assert!(url1.contains("X-Amz-Date=20260610T123456Z"));
        assert!(url1.contains("X-Amz-Expires=600"));
        assert!(url1.contains("X-Amz-SignedHeaders=host"));
        // Signature is 64 lowercase hex chars.
        let sig = url1.rsplit("X-Amz-Signature=").next().unwrap();
        assert_eq!(sig.len(), 64, "sig should be 32-byte hex");
        assert!(sig.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    /// Different HTTP methods produce different signatures (the method is in
    /// the canonical request), so a GET URL can't be replayed as a PUT.
    #[test]
    fn presign_method_binds_signature() {
        let cfg = test_config();
        let t = UNIX_EPOCH + Duration::from_secs(1_781_094_896);
        let key = R2Config::collection_key("dev");
        let put = cfg.presign(PresignMethod::Put, &key, DEFAULT_PRESIGN_TTL, t);
        let get = cfg.presign(PresignMethod::Get, &key, DEFAULT_PRESIGN_TTL, t);
        let put_sig = put.rsplit("X-Amz-Signature=").next().unwrap();
        let get_sig = get.rsplit("X-Amz-Signature=").next().unwrap();
        assert_ne!(put_sig, get_sig);
    }

    #[test]
    fn debug_redacts_secret() {
        let cfg = test_config();
        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains("wJalrXUtnFEMI"), "secret must not appear in Debug");
        assert!(dbg.contains("<redacted>"));
    }

    /// LIVE round-trip against the real R2 bucket — proves the presigned URLs
    /// are actually ACCEPTED by R2 (a passing deterministic unit test only
    /// proves we sign *consistently*, not *correctly*). `#[ignore]` so it
    /// never runs in `make validate` / CI (which must be hermetic — see
    /// CLAUDE.md). Run manually after `source ../../.r2.env`:
    ///
    /// ```sh
    /// cargo test --features network --lib web_server::r2::tests::live_round_trip -- --ignored --nocapture
    /// ```
    ///
    /// PUT a tiny payload, HEAD it (capture ETag), GET it back, assert bytes
    /// match, then conditional-PUT with a stale If-Match and assert 412.
    #[tokio::test]
    #[ignore = "live R2 network round-trip; run manually with R2 creds in env"]
    async fn live_round_trip() {
        let Some(cfg) = R2Config::from_env() else {
            eprintln!("SKIP live_round_trip: R2_* env vars not set");
            return;
        };
        let key = R2Config::collection_key("dev");
        let now = SystemTime::now();
        let body = b"deepscry-live-probe".to_vec();
        let client = reqwest::Client::new();

        // PUT
        let put_url = cfg.presign(PresignMethod::Put, &key, DEFAULT_PRESIGN_TTL, now);
        let resp = client
            .put(&put_url)
            .header("content-type", "application/gzip")
            .body(body.clone())
            .send()
            .await
            .expect("PUT send");
        assert!(
            resp.status().is_success(),
            "PUT failed: {} {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        eprintln!("PUT ok, etag={etag:?}");

        // GET
        let get_url = cfg.presign(PresignMethod::Get, &key, DEFAULT_PRESIGN_TTL, now);
        let resp = client.get(&get_url).send().await.expect("GET send");
        assert!(resp.status().is_success(), "GET failed: {}", resp.status());
        let got = resp.bytes().await.expect("GET body");
        assert_eq!(got.as_ref(), body.as_slice(), "round-trip bytes differ");
        eprintln!("GET ok, {} bytes match", got.len());

        // Conditional PUT with a stale ETag → expect 412 Precondition Failed.
        let put_url2 = cfg.presign(PresignMethod::Put, &key, DEFAULT_PRESIGN_TTL, now);
        let resp = client
            .put(&put_url2)
            .header("content-type", "application/gzip")
            .header("if-match", "\"0000000000000000000000000000000000\"")
            .body(b"should-be-rejected".to_vec())
            .send()
            .await
            .expect("conditional PUT send");
        eprintln!("conditional PUT with stale If-Match → {}", resp.status());
        assert_eq!(
            resp.status().as_u16(),
            412,
            "stale If-Match should be rejected with 412"
        );
    }
}
