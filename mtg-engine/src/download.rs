//! Card Image Downloader
//!
//! Downloads card images from Scryfall API for offline use.
//! Images are stored locally and can be served by the WASM GUI.
//!
//! ## Usage
//!
//! ```bash
//! # Download images for all cards in cardsfolder
//! mtg download
//!
//! # Download specific image sizes
//! mtg download --sizes normal,small
//!
//! # Download only cards in a specific deck
//! mtg download --deck decks/burn.dck
//! ```
//!
//! ## Scryfall API
//!
//! Images are fetched using the Scryfall named card API:
//! `https://api.scryfall.com/cards/named?exact=<name>&format=image&version=<size>`
//!
//! Available versions: small (146x204), normal (488x680), large, png, art_crop, border_crop

use crate::{MtgError, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::Semaphore;

/// Image sizes available from Scryfall
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageSize {
    /// 146x204 pixels - good for thumbnails
    Small,
    /// 488x680 pixels - standard size for display
    Normal,
}

impl ImageSize {
    /// Get the Scryfall API version string
    pub fn api_version(&self) -> &'static str {
        match self {
            ImageSize::Small => "small",
            ImageSize::Normal => "normal",
        }
    }

    /// Get the subfolder name for this size
    pub fn folder_name(&self) -> &'static str {
        match self {
            ImageSize::Small => "small",
            ImageSize::Normal => "normal",
        }
    }

    /// Parse from string
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "small" => Some(ImageSize::Small),
            "normal" => Some(ImageSize::Normal),
            _ => None,
        }
    }
}

/// Get the first-letter subdirectory for a card name (like cardsfolder structure)
///
/// Returns lowercase first letter for a-z, or "_" for numbers/symbols
pub fn first_letter_subdir(card_name: &str) -> String {
    let first_char = card_name.chars().next().unwrap_or('_');
    if first_char.is_ascii_alphabetic() {
        first_char.to_ascii_lowercase().to_string()
    } else {
        "_".to_string()
    }
}

/// Configuration for the download operation
#[derive(Debug)]
pub struct DownloadConfig {
    /// Output directory for images (default: images/)
    pub output_dir: PathBuf,
    /// Card names to download (if empty, downloads all)
    pub card_names: Vec<String>,
    /// Image sizes to download
    pub sizes: Vec<ImageSize>,
    /// Maximum concurrent downloads
    pub concurrency: usize,
    /// Skip cards that already have images downloaded
    pub skip_existing: bool,
    /// Rate limit delay between requests (milliseconds)
    pub rate_limit_ms: u64,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("images"),
            card_names: Vec::new(),
            sizes: vec![ImageSize::Small, ImageSize::Normal],
            concurrency: 10, // Scryfall recommends max 10 concurrent requests
            skip_existing: true,
            rate_limit_ms: 100, // 10 requests/second to be nice to Scryfall
        }
    }
}

/// Download card images from Scryfall
pub struct ImageDownloader {
    config: DownloadConfig,
    client: reqwest::Client,
}

impl ImageDownloader {
    /// Create a new downloader with the given configuration
    pub fn new(config: DownloadConfig) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("mtg-forge-rs/0.1 (https://github.com/your-repo)")
            .build()
            .expect("Failed to create HTTP client");

        Self { config, client }
    }

    /// Build Scryfall URL for a card name and size
    fn build_url(card_name: &str, size: ImageSize) -> String {
        // URL-encode the card name
        let encoded_name = urlencoding::encode(card_name);
        format!(
            "https://api.scryfall.com/cards/named?exact={}&format=image&version={}",
            encoded_name,
            size.api_version()
        )
    }

    /// Get the local file path for a card image
    ///
    /// Uses first-letter subdirectories like cardsfolder:
    /// `images/small/l/Lightning Bolt.jpg`
    fn get_image_path(&self, card_name: &str, size: ImageSize) -> PathBuf {
        // Sanitize card name for filesystem (replace special characters)
        let safe_name: String = card_name
            .chars()
            .map(|c| match c {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                _ => c,
            })
            .collect();

        let first_letter = first_letter_subdir(card_name);

        self.config
            .output_dir
            .join(size.folder_name())
            .join(&first_letter)
            .join(format!("{}.jpg", safe_name))
    }

    /// Check if an image already exists locally
    async fn image_exists(&self, card_name: &str, size: ImageSize) -> bool {
        let path = self.get_image_path(card_name, size);
        fs::metadata(&path).await.is_ok()
    }

    /// Download all configured card images
    pub async fn download_all(&self) -> Result<DownloadStats> {
        let mut stats = DownloadStats::default();

        log::info!(
            "Starting download: {} cards, {} sizes, {} concurrent, {}ms rate limit",
            self.config.card_names.len(),
            self.config.sizes.len(),
            self.config.concurrency,
            self.config.rate_limit_ms
        );

        // Create output directories
        for size in &self.config.sizes {
            let dir = self.config.output_dir.join(size.folder_name());
            log::debug!("Creating output directory: {:?}", dir);
            fs::create_dir_all(&dir).await.map_err(|e| {
                MtgError::IoError(std::io::Error::other(format!(
                    "Failed to create output directory {:?}: {}",
                    dir, e
                )))
            })?;
        }

        // Build download tasks list
        log::info!("Checking for existing images...");
        let mut tasks: Vec<(String, ImageSize)> = Vec::new();
        for card_name in &self.config.card_names {
            for size in &self.config.sizes {
                if self.config.skip_existing && self.image_exists(card_name, *size).await {
                    stats.skipped += 1;
                    log::trace!("Skipping existing: {} ({:?})", card_name, size);
                    continue;
                }
                tasks.push((card_name.clone(), *size));
            }
        }

        let total = tasks.len();
        log::info!(
            "Task list built: {} to download, {} skipped (already exist)",
            total,
            stats.skipped
        );
        if total == 0 {
            log::info!("No images to download (all already exist or no cards specified)");
            return Ok(stats);
        }

        // Set up progress tracking
        let multi_progress = MultiProgress::new();
        let progress_bar = multi_progress.add(ProgressBar::new(total as u64));
        progress_bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
                .expect("Invalid progress bar template")
                .progress_chars("#>-"),
        );

        // Set up concurrency limiter
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency));
        let rate_limit_ms = self.config.rate_limit_ms;

        // Process downloads with bounded concurrency
        // Rate limit is applied between spawning tasks, not within them
        let mut handles = Vec::new();
        for (card_name, size) in tasks {
            // Rate limiting: sleep between spawning tasks to spread out requests
            tokio::time::sleep(tokio::time::Duration::from_millis(rate_limit_ms)).await;

            let client = self.client.clone();
            let permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();
            let output_dir = self.config.output_dir.clone();
            let pb = progress_bar.clone();

            let handle = tokio::spawn(async move {
                let url = Self::build_url(&card_name, size);
                let path = {
                    let safe_name: String = card_name
                        .chars()
                        .map(|c| match c {
                            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                            _ => c,
                        })
                        .collect();
                    let first_letter = first_letter_subdir(&card_name);
                    output_dir
                        .join(size.folder_name())
                        .join(&first_letter)
                        .join(format!("{}.jpg", safe_name))
                };

                log::debug!("Downloading: {} ({:?}) -> {:?}", card_name, size, path);

                // Ensure directory exists
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent).await;
                }

                let result = async {
                    // Retry loop with exponential backoff for rate limiting
                    let max_retries = 5;
                    let mut retry_delay_ms = 1000u64; // Start with 1 second

                    for attempt in 0..=max_retries {
                        let response = client
                            .get(&url)
                            .send()
                            .await
                            .map_err(|e| format!("HTTP error for '{}': {}", card_name, e))?;

                        let status = response.status();

                        // Handle rate limiting (429)
                        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                            if attempt < max_retries {
                                log::debug!(
                                    "Rate limited for '{}', retry {} in {}ms",
                                    card_name,
                                    attempt + 1,
                                    retry_delay_ms
                                );
                                tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay_ms)).await;
                                retry_delay_ms *= 2; // Exponential backoff
                                continue;
                            }
                            return Err(format!(
                                "HTTP 429 Too Many Requests for '{}' (after {} retries)",
                                card_name, max_retries
                            ));
                        }

                        if !status.is_success() {
                            return Err(format!("HTTP {} for '{}'", status, card_name));
                        }

                        let bytes = response
                            .bytes()
                            .await
                            .map_err(|e| format!("Read error for '{}': {}", card_name, e))?;

                        log::trace!("Downloaded {} bytes for '{}' ({:?})", bytes.len(), card_name, size);

                        fs::write(&path, &bytes)
                            .await
                            .map_err(|e| format!("Write error for '{}': {}", card_name, e))?;

                        return Ok::<_, String>(());
                    }
                    Err(format!("Failed to download '{}' after retries", card_name))
                }
                .await;

                pb.inc(1);
                drop(permit);
                result
            });

            handles.push(handle);
        }

        // Wait for all downloads to complete with periodic progress logging
        let start_time = std::time::Instant::now();
        let mut last_log_time = start_time;
        let log_interval = std::time::Duration::from_secs(30);

        for (i, handle) in handles.into_iter().enumerate() {
            match handle.await {
                Ok(Ok(())) => stats.downloaded += 1,
                Ok(Err(e)) => {
                    stats.failed += 1;
                    log::warn!("{}", e);
                }
                Err(e) => {
                    stats.failed += 1;
                    log::error!("Task panic: {}", e);
                }
            }

            // Periodic progress logging
            let now = std::time::Instant::now();
            if now.duration_since(last_log_time) >= log_interval {
                let elapsed = now.duration_since(start_time);
                let completed = stats.downloaded + stats.failed;
                let rate = completed as f64 / elapsed.as_secs_f64();
                let remaining = total - (i + 1);
                let eta_secs = if rate > 0.0 { remaining as f64 / rate } else { 0.0 };
                log::info!(
                    "Progress: {}/{} ({:.1}%), {:.1} img/sec, ETA: {:.0}s",
                    i + 1,
                    total,
                    (i + 1) as f64 / total as f64 * 100.0,
                    rate,
                    eta_secs
                );
                last_log_time = now;
            }
        }

        let elapsed = start_time.elapsed();
        log::info!(
            "Download complete in {:.1}s. Rate: {:.1} img/sec",
            elapsed.as_secs_f64(),
            (stats.downloaded + stats.failed) as f64 / elapsed.as_secs_f64()
        );

        progress_bar.finish_with_message("Done!");
        Ok(stats)
    }
}

/// Statistics from a download operation
#[derive(Debug, Default)]
pub struct DownloadStats {
    /// Number of images successfully downloaded
    pub downloaded: usize,
    /// Number of images skipped (already exist)
    pub skipped: usize,
    /// Number of images that failed to download
    pub failed: usize,
}

impl std::fmt::Display for DownloadStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Downloaded: {}, Skipped: {}, Failed: {}",
            self.downloaded, self.skipped, self.failed
        )
    }
}

/// Load card names from cardsfolder
pub async fn load_card_names_from_cardsfolder(cardsfolder: &Path) -> Result<Vec<String>> {
    let mut names = HashSet::new();

    // Walk the cardsfolder directory
    let mut entries = fs::read_dir(cardsfolder).await.map_err(|e| {
        MtgError::IoError(std::io::Error::other(format!(
            "Failed to read cardsfolder {:?}: {}",
            cardsfolder, e
        )))
    })?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| MtgError::IoError(std::io::Error::other(format!("Failed to read directory entry: {}", e))))?
    {
        let path = entry.path();
        if path.is_dir() {
            // Recurse into subdirectories (a-z folders)
            let mut subentries = fs::read_dir(&path).await.map_err(|e| {
                MtgError::IoError(std::io::Error::other(format!(
                    "Failed to read subdirectory {:?}: {}",
                    path, e
                )))
            })?;

            while let Some(subentry) = subentries.next_entry().await.map_err(|e| {
                MtgError::IoError(std::io::Error::other(format!("Failed to read directory entry: {}", e)))
            })? {
                let subpath = subentry.path();
                if subpath.extension().is_some_and(|ext| ext == "txt") {
                    if let Some(name) = extract_card_name(&subpath).await {
                        names.insert(name);
                    }
                }
            }
        } else if path.extension().is_some_and(|ext| ext == "txt") {
            if let Some(name) = extract_card_name(&path).await {
                names.insert(name);
            }
        }
    }

    let mut result: Vec<String> = names.into_iter().collect();
    result.sort();
    Ok(result)
}

/// Extract card name from a card .txt file
async fn extract_card_name(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).await.ok()?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("Name:") {
            return Some(name.trim().to_string());
        }
    }
    None
}

/// Load card names from a deck file
pub async fn load_card_names_from_deck(deck_path: &Path) -> Result<Vec<String>> {
    let content = fs::read_to_string(deck_path).await.map_err(|e| {
        MtgError::IoError(std::io::Error::other(format!(
            "Failed to read deck file {:?}: {}",
            deck_path, e
        )))
    })?;

    let mut names = HashSet::new();
    let mut in_metadata_section = false;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Handle section headers
        if line.starts_with('[') && line.ends_with(']') {
            let section = &line[1..line.len() - 1].to_lowercase();
            in_metadata_section = section == "metadata";
            continue;
        }

        // Skip metadata section lines (Name=..., Description=...)
        if in_metadata_section || line.contains('=') {
            continue;
        }

        // Deck format: "N CardName" or "CardName"
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        let card_name = if parts.len() == 2 && parts[0].parse::<u32>().is_ok() {
            parts[1].trim()
        } else {
            line
        };

        // Handle set code suffix: "Card Name|SET"
        let card_name = card_name.split('|').next().unwrap_or(card_name).trim();

        if !card_name.is_empty() {
            names.insert(card_name.to_string());
        }
    }

    let mut result: Vec<String> = names.into_iter().collect();
    result.sort();
    Ok(result)
}

// We need urlencoding for the card names
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut result = String::with_capacity(s.len() * 3);
        for byte in s.bytes() {
            match byte {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(byte as char);
                }
                b' ' => result.push_str("%20"),
                _ => {
                    result.push('%');
                    result.push_str(&format!("{:02X}", byte));
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_url() {
        let url = ImageDownloader::build_url("Lightning Bolt", ImageSize::Normal);
        assert_eq!(
            url,
            "https://api.scryfall.com/cards/named?exact=Lightning%20Bolt&format=image&version=normal"
        );
    }

    #[test]
    fn test_url_encoding() {
        assert_eq!(urlencoding::encode("Lightning Bolt"), "Lightning%20Bolt");
        assert_eq!(
            urlencoding::encode("Jace, the Mind Sculptor"),
            "Jace%2C%20the%20Mind%20Sculptor"
        );
    }

    #[test]
    fn test_image_size_from_str() {
        assert_eq!(ImageSize::parse("small"), Some(ImageSize::Small));
        assert_eq!(ImageSize::parse("NORMAL"), Some(ImageSize::Normal));
        assert_eq!(ImageSize::parse("invalid"), None);
    }

    #[test]
    fn test_first_letter_subdir() {
        assert_eq!(first_letter_subdir("Lightning Bolt"), "l");
        assert_eq!(first_letter_subdir("Ancestral Recall"), "a");
        assert_eq!(first_letter_subdir("Zealous Conscripts"), "z");
        // Uppercase should become lowercase
        assert_eq!(first_letter_subdir("BLACK LOTUS"), "b");
        // Numbers/symbols go to "_"
        assert_eq!(first_letter_subdir("1996 World Champion"), "_");
        assert_eq!(first_letter_subdir("+2 Mace"), "_");
        // Empty string edge case
        assert_eq!(first_letter_subdir(""), "_");
    }
}
