// help_dialog.js — shared help modal (mtg-1vwpd DRY extraction).
//
// Both game pages (web/tui_game.html and web/native_game.html) render the same
// help overlay: a `#help-dialog` + `#help-dialog-overlay` pair whose body is the
// shared Rust help text (`tui_get_help_text`), dismissed on any keypress or a
// click on the overlay/dialog. This module is the single source of truth for
// that behavior so the two pages can never drift.
//
// The help text itself is supplied by the caller (a closure over the page's WASM
// `tui_get_help_text` import) so this module stays renderer- and import-agnostic.
//
// Usage:
//   import { installHelpDialog } from './help_dialog.js';
//   const help = installHelpDialog({ getHelpText: () => tui_get_help_text() });
//   window.showHelpDialog = help.show;   // WASM key handler calls this
//
// DOM contract (must exist in the page): elements with ids
//   help-dialog, help-dialog-overlay, help-dialog-content.

/**
 * Wire up the shared help dialog.
 *
 * @param {Object} opts
 * @param {() => string} opts.getHelpText - returns the help body text (typically
 *   a closure over the page's `tui_get_help_text` WASM import). If it throws or
 *   is unavailable, a fallback message is shown.
 * @returns {{ show: () => void, hide: () => void }}
 */
export function installHelpDialog({ getHelpText }) {
    function hide() {
        const dialog = document.getElementById('help-dialog');
        const overlay = document.getElementById('help-dialog-overlay');
        if (!dialog || !overlay) return;
        dialog.classList.remove('show');
        overlay.classList.remove('show');
        if (dialog._helpKeyHandler) {
            document.removeEventListener('keydown', dialog._helpKeyHandler, true);
            dialog._helpKeyHandler = null;
        }
        overlay.onclick = null;
        dialog.onclick = null;
    }

    function show() {
        const dialog = document.getElementById('help-dialog');
        const overlay = document.getElementById('help-dialog-overlay');
        const content = document.getElementById('help-dialog-content');
        if (!dialog || !overlay || !content) return;

        let text = 'Help text not available (WASM not loaded)';
        try {
            const t = typeof getHelpText === 'function' ? getHelpText() : null;
            if (typeof t === 'string' && t.length > 0) text = t;
        } catch (e) {
            // Keep the fallback text.
        }
        content.textContent = text;

        dialog.classList.add('show');
        overlay.classList.add('show');

        // Close on any key (capture phase + stopPropagation so the close
        // keystroke is not also interpreted as a game command).
        const handleHelpKey = function (e) {
            hide();
            e.preventDefault();
            e.stopPropagation();
        };
        dialog._helpKeyHandler = handleHelpKey;
        document.addEventListener('keydown', handleHelpKey, true);
        overlay.onclick = hide;
        dialog.onclick = hide;
    }

    return { show, hide };
}
