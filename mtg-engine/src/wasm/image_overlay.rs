/// Card Image Overlay System for WASM GUI
///
/// This module provides card image support by overlaying DOM <img> elements
/// on top of the RatZilla terminal rendering. This allows us to:
/// 1. Keep all TUI rendering code intact (shared with native)
/// 2. Progressively enhance with card images
/// 3. Fall back gracefully when images fail to load
///
/// Architecture:
/// - Images are fetched from Gatherer (primary) with Scryfall as fallback
/// - Images are positioned using CSS over terminal cells
/// - RatZilla cells are 10px wide x 20px tall (CELL_WIDTH_PX x CELL_HEIGHT_PX)
use wasm_bindgen::prelude::*;
use web_sys::{Document, HtmlElement, HtmlImageElement};

/// Constants for cell-to-pixel conversion
/// These match RatZilla's DomBackend settings
const CELL_WIDTH_PX: f64 = 10.0;
const CELL_HEIGHT_PX: f64 = 20.0;

/// Image version to fetch from Scryfall (used as fallback)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageVersion {
    /// Small (146x204) - for battlefield view
    Small,
    /// Normal (488x680) - for detail view
    Normal,
    /// Art crop - just the artwork
    ArtCrop,
}

/// Build Gatherer image URL for a card by name
///
/// Gatherer is the official WotC card database and provides images by card name.
/// Images are ~223x310 pixels (medium size only).
///
/// # Arguments
/// * `card_name` - The card name (will be URL-encoded)
///
/// # Returns
/// Full URL to Gatherer image handler
pub fn gatherer_url(card_name: &str) -> String {
    // URL-encode the card name for the query parameter
    let encoded_name = js_sys::encode_uri_component(card_name);
    format!(
        "https://gatherer.wizards.com/Handlers/Image.ashx?name={}&type=card",
        encoded_name
    )
}

/// Build Scryfall image URL for a card by set code and collector number (fallback)
///
/// # Arguments
/// * `set_code` - Three-letter set code (e.g. "LEA", "ARN")
/// * `collector_number` - Collector number as string
/// * `version` - Image size/type
///
/// # Returns
/// Full URL to Scryfall image API
pub fn scryfall_url(set_code: &str, collector_number: &str, version: ImageVersion) -> String {
    let version_str = match version {
        ImageVersion::Small => "small",
        ImageVersion::Normal => "normal",
        ImageVersion::ArtCrop => "art_crop",
    };

    format!(
        "https://api.scryfall.com/cards/{}/{}?format=image&version={}",
        set_code.to_lowercase(),
        collector_number,
        version_str
    )
}

/// Build Scryfall image URL for a card by name
///
/// Uses Scryfall's named card API endpoint which looks up by exact name.
///
/// # Arguments
/// * `card_name` - Card name (e.g. "Lightning Bolt")
/// * `version` - Image size/type
///
/// # Returns
/// Full URL to Scryfall named card image API
pub fn scryfall_url_by_name(card_name: &str, version: ImageVersion) -> String {
    let version_str = match version {
        ImageVersion::Small => "small",
        ImageVersion::Normal => "normal",
        ImageVersion::ArtCrop => "art_crop",
    };

    // URL-encode the card name
    let encoded_name = url_encode(card_name);
    format!(
        "https://api.scryfall.com/cards/named?exact={}&format=image&version={}",
        encoded_name, version_str
    )
}

/// Build local image URL for a card (from images/ directory)
///
/// Local images are downloaded via `mtg download` command.
/// Uses first-letter subdirectories like cardsfolder structure:
/// `./images/small/l/Lightning Bolt.jpg`
///
/// # Arguments
/// * `card_name` - Card name (e.g. "Lightning Bolt")
/// * `version` - Image size/type
/// * `base_url` - Base URL for images (e.g. "./images" or "/images")
///
/// # Returns
/// Local URL to card image
pub fn local_image_url(card_name: &str, version: ImageVersion, base_url: &str) -> String {
    let version_str = match version {
        ImageVersion::Small => "small",
        ImageVersion::Normal => "normal",
        ImageVersion::ArtCrop => "normal", // No local art_crop, fall back to normal
    };

    // Get first-letter subdirectory (like cardsfolder)
    let first_letter = first_letter_subdir(card_name);

    // Sanitize card name for filesystem
    let safe_name: String = card_name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();

    format!("{}/{}/{}/{}.jpg", base_url, version_str, first_letter, safe_name)
}

/// Get the first-letter subdirectory for a card name (like cardsfolder structure)
///
/// Returns lowercase first letter for a-z, or "_" for numbers/symbols
fn first_letter_subdir(card_name: &str) -> String {
    let first_char = card_name.chars().next().unwrap_or('_');
    if first_char.is_ascii_alphabetic() {
        first_char.to_ascii_lowercase().to_string()
    } else {
        "_".to_string()
    }
}

/// URL-encode a string for use in URLs
fn url_encode(s: &str) -> String {
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

/// Image size dimensions (in pixels)
pub const SMALL_IMAGE_HEIGHT: u32 = 204;
pub const NORMAL_IMAGE_HEIGHT: u32 = 680;

/// Choose the appropriate image version based on rendered size
///
/// Uses the small version when the rendered height is <= the small image height,
/// and normal version for larger displays to avoid pixelation.
///
/// # Arguments
/// * `rendered_height_px` - The height at which the image will be displayed (in pixels)
///
/// # Returns
/// The appropriate ImageVersion for the given display size
pub fn choose_image_version(rendered_height_px: u32) -> ImageVersion {
    if rendered_height_px <= SMALL_IMAGE_HEIGHT {
        ImageVersion::Small
    } else {
        ImageVersion::Normal
    }
}

/// Convert terminal cell coordinates to CSS pixel coordinates
///
/// # Arguments
/// * `col` - Column number (0-based)
/// * `row` - Row number (0-based)
///
/// # Returns
/// (left_px, top_px) tuple for CSS positioning
pub fn cell_to_pixels(col: u16, row: u16) -> (f64, f64) {
    (f64::from(col) * CELL_WIDTH_PX, f64::from(row) * CELL_HEIGHT_PX)
}

/// Manager for DOM image overlays
///
/// This manages <img> elements positioned absolutely over the terminal.
/// Images are created, positioned, and removed as cards move around the battlefield.
pub struct ImageOverlayManager {
    document: Document,
    container_id: String,
    enabled: bool,
}

impl ImageOverlayManager {
    /// Create a new ImageOverlayManager
    ///
    /// # Arguments
    /// * `container_id` - ID of the container element to append images to
    /// * `enabled` - Whether to actually create images (can be toggled by user)
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` error if window or document cannot be accessed.
    pub fn new(container_id: &str, enabled: bool) -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("No window"))?;
        let document = window.document().ok_or_else(|| JsValue::from_str("No document"))?;

        Ok(Self {
            document,
            container_id: container_id.to_string(),
            enabled,
        })
    }

    /// Toggle image overlays on/off
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            // Remove all existing overlays
            self.clear_all_overlays();
        }
    }

    /// Create or update an image overlay for a card
    ///
    /// Uses Gatherer as primary source with Scryfall as fallback on error.
    ///
    /// # Arguments
    /// * `card_id` - Unique identifier for this card instance
    /// * `col` - Column position in terminal cells
    /// * `row` - Row position in terminal cells
    /// * `width` - Width in terminal cells
    /// * `height` - Height in terminal cells
    /// * `card_name` - Card name for Gatherer lookup
    /// * `fallback_url` - Optional Scryfall URL to use if Gatherer fails
    ///
    /// This creates an <img> element with:
    /// - ID: `card-image-{card_id}`
    /// - Absolute positioning over terminal
    /// - Z-index to appear above terminal but below UI elements
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` error if DOM element creation or style manipulation fails.
    #[allow(clippy::too_many_arguments)]
    pub fn set_card_image_with_fallback(
        &self,
        card_id: &str,
        col: u16,
        row: u16,
        width: u16,
        height: u16,
        card_name: &str,
        fallback_url: Option<&str>,
    ) -> Result<(), JsValue> {
        if !self.enabled {
            return Ok(());
        }

        let img_id = format!("card-image-{}", card_id);
        let primary_url = gatherer_url(card_name);

        // Check if image already exists
        let img = if let Some(existing) = self.document.get_element_by_id(&img_id) {
            existing.dyn_into::<HtmlImageElement>()?
        } else {
            // Create new image element
            let img = self.document.create_element("img")?.dyn_into::<HtmlImageElement>()?;
            img.set_id(&img_id);
            img.set_class_name("card-overlay-image");

            // Set primary source (Gatherer)
            img.set_src(&primary_url);

            // Add error handler that falls back to Scryfall
            let fallback = fallback_url.map(|s| s.to_string());
            let primary_url_clone = primary_url;
            let error_callback = Closure::wrap(Box::new(move |event: web_sys::Event| {
                if let Some(ref fallback_src) = fallback {
                    // Try fallback URL
                    if let Some(target) = event.target() {
                        if let Ok(img_elem) = target.dyn_into::<HtmlImageElement>() {
                            // Only fallback if we haven't already (check current src)
                            let current_src = img_elem.src();
                            if !current_src.contains("api.scryfall.com") {
                                web_sys::console::log_1(&JsValue::from_str(&format!(
                                    "Gatherer failed for {}, trying Scryfall fallback",
                                    primary_url_clone
                                )));
                                img_elem.set_src(fallback_src);
                            } else {
                                web_sys::console::warn_1(&JsValue::from_str(
                                    "Both Gatherer and Scryfall failed for image",
                                ));
                            }
                        }
                    }
                } else {
                    web_sys::console::warn_1(&JsValue::from_str(&format!(
                        "Failed to load card image from Gatherer: {}",
                        primary_url_clone
                    )));
                }
            }) as Box<dyn FnMut(_)>);

            img.set_onerror(Some(error_callback.as_ref().unchecked_ref()));
            error_callback.forget(); // Keep callback alive

            // Append to container
            if let Some(container) = self.document.get_element_by_id(&self.container_id) {
                container.append_child(&img)?;
            }

            img
        };

        // Update position and size
        let (left_px, top_px) = cell_to_pixels(col, row);
        let width_px = f64::from(width) * CELL_WIDTH_PX;
        let height_px = f64::from(height) * CELL_HEIGHT_PX;

        // Cast to HtmlElement to access style
        let html_elem: &HtmlElement = img.as_ref();
        let style = html_elem.style();
        style.set_property("position", "absolute")?;
        style.set_property("left", &format!("{}px", left_px))?;
        style.set_property("top", &format!("{}px", top_px))?;
        style.set_property("width", &format!("{}px", width_px))?;
        style.set_property("height", &format!("{}px", height_px))?;
        style.set_property("object-fit", "contain")?; // Preserve aspect ratio
        style.set_property("pointer-events", "none")?; // Don't block terminal interactions
        style.set_property("z-index", "10")?; // Above terminal, below UI controls

        Ok(())
    }

    /// Create or update an image overlay for a card (legacy API)
    ///
    /// # Arguments
    /// * `card_id` - Unique identifier for this card instance
    /// * `col` - Column position in terminal cells
    /// * `row` - Row position in terminal cells
    /// * `width` - Width in terminal cells
    /// * `height` - Height in terminal cells
    /// * `image_url` - URL to card image
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` error if DOM element creation or style manipulation fails.
    pub fn set_card_image(
        &self,
        card_id: &str,
        col: u16,
        row: u16,
        width: u16,
        height: u16,
        image_url: &str,
    ) -> Result<(), JsValue> {
        if !self.enabled {
            return Ok(());
        }

        let img_id = format!("card-image-{}", card_id);

        // Check if image already exists
        let img = if let Some(existing) = self.document.get_element_by_id(&img_id) {
            existing.dyn_into::<HtmlImageElement>()?
        } else {
            // Create new image element
            let img = self.document.create_element("img")?.dyn_into::<HtmlImageElement>()?;
            img.set_id(&img_id);
            img.set_class_name("card-overlay-image");

            // Set src (will trigger loading)
            img.set_src(image_url);

            // Add error handler
            let url_clone = image_url.to_string();
            let error_callback = Closure::wrap(Box::new(move |_event: web_sys::Event| {
                web_sys::console::warn_1(&JsValue::from_str(&format!("Failed to load card image: {}", url_clone)));
            }) as Box<dyn FnMut(_)>);

            img.set_onerror(Some(error_callback.as_ref().unchecked_ref()));
            error_callback.forget(); // Keep callback alive

            // Append to container
            if let Some(container) = self.document.get_element_by_id(&self.container_id) {
                container.append_child(&img)?;
            }

            img
        };

        // Update position and size
        let (left_px, top_px) = cell_to_pixels(col, row);
        let width_px = f64::from(width) * CELL_WIDTH_PX;
        let height_px = f64::from(height) * CELL_HEIGHT_PX;

        // Cast to HtmlElement to access style
        let html_elem: &HtmlElement = img.as_ref();
        let style = html_elem.style();
        style.set_property("position", "absolute")?;
        style.set_property("left", &format!("{}px", left_px))?;
        style.set_property("top", &format!("{}px", top_px))?;
        style.set_property("width", &format!("{}px", width_px))?;
        style.set_property("height", &format!("{}px", height_px))?;
        style.set_property("object-fit", "contain")?; // Preserve aspect ratio
        style.set_property("pointer-events", "none")?; // Don't block terminal interactions
        style.set_property("z-index", "10")?; // Above terminal, below UI controls

        Ok(())
    }

    /// Remove an image overlay for a card
    ///
    /// # Arguments
    /// * `card_id` - Unique identifier for this card instance
    ///
    /// # Errors
    ///
    /// This function currently always succeeds, but returns Result for API consistency.
    pub fn remove_overlay(&self, card_id: &str) -> Result<(), JsValue> {
        let img_id = format!("card-image-{}", card_id);
        if let Some(img) = self.document.get_element_by_id(&img_id) {
            img.remove();
        }
        Ok(())
    }

    /// Remove all image overlays
    pub fn clear_all_overlays(&self) {
        // Find all elements with class "card-overlay-image" and remove them
        // Note: we query all and remove individually since web-sys doesn't have a batch remove
        let mut to_remove: Vec<web_sys::Element> = Vec::new();

        // First pass: collect elements to remove
        if let Some(body) = self.document.body() {
            if let Ok(elements) = body.query_selector_all(".card-overlay-image") {
                for i in 0..elements.length() {
                    if let Some(node) = elements.item(i) {
                        // NodeList.item() returns Node, need to cast to Element
                        if let Ok(element) = node.dyn_into::<web_sys::Element>() {
                            to_remove.push(element);
                        }
                    }
                }
            }
        }

        // Second pass: remove collected elements
        for element in to_remove {
            element.remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gatherer_url() {
        let url = gatherer_url("Lightning Bolt");
        assert_eq!(
            url,
            "https://gatherer.wizards.com/Handlers/Image.ashx?name=Lightning%20Bolt&type=card"
        );

        let url = gatherer_url("Swamp");
        assert_eq!(
            url,
            "https://gatherer.wizards.com/Handlers/Image.ashx?name=Swamp&type=card"
        );
    }

    #[test]
    fn test_scryfall_url() {
        let url = scryfall_url("LEA", "231", ImageVersion::Small);
        assert_eq!(url, "https://api.scryfall.com/cards/lea/231?format=image&version=small");

        let url = scryfall_url("ARN", "1", ImageVersion::Normal);
        assert_eq!(url, "https://api.scryfall.com/cards/arn/1?format=image&version=normal");
    }

    #[test]
    fn test_scryfall_url_by_name() {
        let url = scryfall_url_by_name("Lightning Bolt", ImageVersion::Normal);
        assert_eq!(
            url,
            "https://api.scryfall.com/cards/named?exact=Lightning%20Bolt&format=image&version=normal"
        );

        let url = scryfall_url_by_name("Jace, the Mind Sculptor", ImageVersion::Small);
        assert_eq!(
            url,
            "https://api.scryfall.com/cards/named?exact=Jace%2C%20the%20Mind%20Sculptor&format=image&version=small"
        );
    }

    #[test]
    fn test_local_image_url() {
        // Tests first-letter subdirectory structure
        let url = local_image_url("Lightning Bolt", ImageVersion::Small, "./images");
        assert_eq!(url, "./images/small/l/Lightning Bolt.jpg");

        let url = local_image_url("Jace, the Mind Sculptor", ImageVersion::Normal, "/images");
        assert_eq!(url, "/images/normal/j/Jace, the Mind Sculptor.jpg");

        // Test special character sanitization
        let url = local_image_url("Who/What/When/Where/Why", ImageVersion::Small, "./images");
        assert_eq!(url, "./images/small/w/Who_What_When_Where_Why.jpg");

        // Test numeric prefix goes to "_" directory
        let url = local_image_url("1996 World Champion", ImageVersion::Normal, "./images");
        assert_eq!(url, "./images/normal/_/1996 World Champion.jpg");
    }

    #[test]
    fn test_cell_to_pixels() {
        assert_eq!(cell_to_pixels(0, 0), (0.0, 0.0));
        assert_eq!(cell_to_pixels(10, 5), (100.0, 100.0));
        assert_eq!(cell_to_pixels(1, 1), (10.0, 20.0));
    }
}
