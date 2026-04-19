//! Card image display for native TUI using ratatui-image
//!
//! Provides terminal-native image rendering in the card details pane,
//! supporting Sixel, Kitty, iTerm2 protocols with automatic detection
//! and halfblock fallback for unsupported terminals.
//!
//! Images are loaded from the local `images/` directory (same structure
//! as the web image overlay and `mtg download` command).

use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui::Frame;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::StatefulImage;
use std::path::{Path, PathBuf};

use crate::core::CardId;

/// Manages card image loading and terminal-native rendering.
///
/// Initialized once at controller startup with terminal protocol detection.
/// Caches the most recently loaded image to avoid re-loading on every frame.
pub struct CardImageState {
    /// Protocol picker (detects terminal capabilities + font size)
    picker: Picker,
    /// Currently loaded image protocol state (for StatefulImage widget)
    current_image: Option<StatefulProtocol>,
    /// Card ID of the currently loaded image (to avoid redundant loads)
    current_card_id: Option<CardId>,
    /// Base directory for local card images
    images_dir: PathBuf,
    /// Whether image support is available (false if protocol detection failed)
    available: bool,
}

impl CardImageState {
    /// Create a new `CardImageState` by querying the terminal for protocol support.
    ///
    /// This queries stdio for terminal capabilities (Sixel, Kitty, iTerm2)
    /// and font size. Falls back to halfblocks if no protocol is detected.
    /// If even that fails, marks images as unavailable.
    pub fn new() -> Self {
        let (picker, available) = match Picker::from_query_stdio() {
            Ok(picker) => (picker, true),
            Err(e) => {
                log::debug!("Terminal image protocol detection failed: {e}, using halfblocks");
                (Picker::halfblocks(), true)
            }
        };

        // Look for images directory relative to CWD
        let images_dir = PathBuf::from("images");

        Self {
            picker,
            current_image: None,
            current_card_id: None,
            images_dir,
            available,
        }
    }

    /// Check if image display is available
    pub fn is_available(&self) -> bool {
        self.available && self.images_dir.exists()
    }

    /// Update the displayed image if the selected card changed.
    ///
    /// Loads the image from disk only when the card ID changes.
    /// Returns true if an image is ready to render.
    pub fn update_for_card(&mut self, card_id: Option<CardId>, card_name: Option<&str>) -> bool {
        if !self.available {
            return false;
        }

        // If card didn't change, keep current image
        if card_id == self.current_card_id {
            return self.current_image.is_some();
        }

        self.current_card_id = card_id;
        self.current_image = None;

        let Some(name) = card_name else {
            return false;
        };

        // Try to load from local images directory
        if let Some(img) = self.load_card_image(name) {
            self.current_image = Some(self.picker.new_resize_protocol(img));
            return true;
        }

        false
    }

    /// Render the card image into the given area.
    ///
    /// Should be called during frame rendering. The image is scaled to fit
    /// the available area while preserving aspect ratio.
    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        if let Some(ref mut protocol) = self.current_image {
            let image_widget = StatefulImage::default();
            f.render_stateful_widget(image_widget, area, protocol);
        }
    }

    /// Try to load a card image from the local images directory.
    ///
    /// Searches for images in the same directory structure used by
    /// `mtg download` and the web image overlay:
    /// `images/small/{first_letter}/{Card Name}.jpg`
    /// `images/normal/{first_letter}/{Card Name}.jpg`
    fn load_card_image(&self, card_name: &str) -> Option<DynamicImage> {
        let first_letter = card_name.chars().next()?.to_uppercase().next()?.to_string();

        // Try small first (faster to render), then normal
        for version in &["small", "normal"] {
            let path = self
                .images_dir
                .join(version)
                .join(&first_letter)
                .join(format!("{}.jpg", card_name));

            if let Some(img) = try_load_image(&path) {
                return Some(img);
            }
        }

        None
    }
}

/// Attempt to load an image file, returning None on any error.
fn try_load_image(path: &Path) -> Option<DynamicImage> {
    match image::ImageReader::open(path) {
        Ok(reader) => match reader.decode() {
            Ok(img) => Some(img),
            Err(e) => {
                log::debug!("Failed to decode image {}: {e}", path.display());
                None
            }
        },
        Err(_) => None, // File doesn't exist, silently skip
    }
}
