// prefs.js — ONE shared, structured form-value-remembering helper for the
// DeepScry web frontend (lobby + both launchers).
//
// WHY THIS EXISTS
// ---------------
// The browser's native autocomplete can remember plain text <input>s, but it
// CANNOT remember <select> dropdowns or radio buttons. The launchers are full
// of those (deck collection, deck, controller, renderer), so their state MUST
// be persisted in JavaScript. Before this file, every page did it differently:
// launcher.html stored a blob under 'mtg-forge-launcher-settings' + the seed
// under 'mtg.mpSeed'; solo_launcher.html remembered ONLY the seed, under a
// DIFFERENT key 'mtg.rngSeed', and forgot its deck/controller picks entirely;
// the lobby stored the username under 'deepscry_display_name'. Same logical
// field, different keys per page — a recipe for silent "it didn't remember"
// bugs.
//
// THE DESIGN (one function PER FIELD TYPE)
// ----------------------------------------
// Every page calls the SAME helper for the same KIND of field, so wiring a
// remembered field is a mechanical check rather than bespoke per-page code:
//
//   rememberTextInput(el, key)    — <input type=text|number|...> text-like
//   rememberSelect(el, key)       — <select> dropdown (returns {restore} for
//                                   selects whose <option>s are populated async)
//   rememberRadioGroup(name, key) — a set of <input type=radio name=...>
//   rememberCheckbox(el, key)     — a single <input type=checkbox>
//
// Each helper, on call: reads the saved value from the single shared store and
// APPLIES it to the element; then registers a change/input listener that WRITES
// the element's current value back on every change.
//
// THE STORE
// ---------
// One localStorage object under the key `deepscry.prefs`, holding a flat map of
// dotted keys. Page-specific fields live under `<page>.<field>`
// (e.g. `solo.p1Deck`, `mp.renderer`); fields that should be the SAME value
// everywhere live under a bare shared key (e.g. `seed` — a seed entered on one
// launcher is the default on the other). Values are read/modified/written as
// structured JSON — never string-munged — per the project's "No Hacky String
// Operations On Structured Data" rule.
//
// localStorage can throw (private mode, disabled storage), so every access is
// guarded and degrades to "no memory" rather than breaking the page.
(function (global) {
  'use strict';

  const STORE_KEY = 'deepscry.prefs';

  // Namespaced field keys. Keeping them as named constants (rather than bare
  // string literals scattered across three pages) is what makes the
  // set→reload→restore round-trip reliable: a typo'd key can't silently break
  // remembering on one page only.
  const KEYS = {
    // Shared across BOTH launchers — a seed entered on one is the default on
    // the other.
    SEED: 'seed',

    // Lobby (index.html).
    USERNAME: 'lobby.username',

    // Multiplayer launcher (launcher.html).
    MP_RENDERER: 'mp.renderer',
    MP_CONTROLLER: 'mp.controller',
    MP_COLLECTION: 'mp.collection',
    MP_DECK: 'mp.deck',
    MP_SHOW_IMAGES: 'mp.showImages',
    MP_IMG_SRC_LOCAL: 'mp.imgSrcLocal',
    MP_IMG_SRC_SCRYFALL: 'mp.imgSrcScryfall',
    MP_IMG_SRC_GATHERER: 'mp.imgSrcGatherer',
    MP_DEBUG: 'mp.debug',

    // Solo launcher (solo_launcher.html).
    SOLO_RENDERER: 'solo.renderer',
    SOLO_P1_COLLECTION: 'solo.p1Collection',
    SOLO_P1_DECK: 'solo.p1Deck',
    SOLO_P2_COLLECTION: 'solo.p2Collection',
    SOLO_P2_DECK: 'solo.p2Deck',
    SOLO_P1_CONTROLLER: 'solo.p1Controller',
    SOLO_P2_CONTROLLER: 'solo.p2Controller',
  };

  // ---------------------------------------------------------------------------
  // Low-level structured store (read/modify/write the whole JSON object).
  // ---------------------------------------------------------------------------

  /** Read+parse the whole prefs object. Returns {} on any failure. */
  function readAll() {
    try {
      const raw = global.localStorage.getItem(STORE_KEY);
      if (!raw) return {};
      const parsed = JSON.parse(raw);
      return (parsed && typeof parsed === 'object') ? parsed : {};
    } catch (e) {
      return {};
    }
  }

  /** Serialize+write the whole prefs object. Silent on failure. */
  function writeAll(obj) {
    try {
      global.localStorage.setItem(STORE_KEY, JSON.stringify(obj));
    } catch (e) { /* ignore — storage disabled / full */ }
  }

  /**
   * Get one pref by key. `fallback` (default undefined) is returned when the
   * key is absent. Values come back exactly as stored (string/number/bool).
   */
  function get(key, fallback) {
    const all = readAll();
    return Object.prototype.hasOwnProperty.call(all, key) ? all[key] : fallback;
  }

  /**
   * Set one pref. Passing `undefined`, `null`, or `''` DELETES the key (so e.g.
   * clearing the seed field removes it and the "(random)"/"(server default)"
   * placeholder behavior is preserved). Other falsey values (false, 0) are
   * stored as-is.
   */
  function set(key, value) {
    const all = readAll();
    if (value === undefined || value === null || value === '') {
      delete all[key];
    } else {
      all[key] = value;
    }
    writeAll(all);
  }

  // ---------------------------------------------------------------------------
  // ONE helper PER FIELD TYPE. Each: apply-saved-on-load + write-on-change.
  // ---------------------------------------------------------------------------

  /**
   * Remember a text-like <input> (type=text, number, search, …).
   * On call: if a value is saved, sets `el.value` to it. Then writes the
   * trimmed value back on every `input`/`change` (blank clears the key).
   */
  function rememberTextInput(el, key) {
    if (!el) return;
    const saved = get(key);
    if (saved !== undefined && saved !== null && saved !== '') {
      el.value = saved;
    }
    const save = function () { set(key, String(el.value).trim()); };
    el.addEventListener('input', save);
    el.addEventListener('change', save);
  }

  /**
   * Remember a <select> dropdown. On call: applies the saved value if it
   * matches an existing <option>, and registers a `change` listener that saves
   * the current value. For selects whose <option>s are populated ASYNCHRONOUSLY
   * (e.g. deck lists fetched from index.json), call the returned `restore()`
   * AFTER the options exist to re-apply the saved choice.
   * @returns {{restore: function(): (string|undefined)}}  restore() applies the
   *   saved value (if it is now a valid option) and returns the value it
   *   applied, or undefined if nothing was applied.
   */
  function rememberSelect(el, key) {
    if (!el) return { restore: function () { return undefined; } };
    const restore = function () {
      const saved = get(key);
      if (saved === undefined || saved === null) return undefined;
      // Only apply if the value is a real option, else leave the page default.
      const ok = Array.prototype.some.call(el.options, function (o) { return o.value === saved; });
      if (ok) { el.value = saved; return saved; }
      return undefined;
    };
    restore();
    el.addEventListener('change', function () { set(key, el.value); });
    return { restore: restore };
  }

  /**
   * Remember a group of radio buttons sharing a `name`. On call: checks the
   * radio whose value was saved (if that radio exists AND is not disabled),
   * and registers `change` listeners that save the newly-selected value.
   * @param {string} name  the shared `name=` attribute of the radio group
   * @param {string} key   the prefs key
   * @param {Document|HTMLElement} [root=document]  scope to search within
   */
  function rememberRadioGroup(name, key, root) {
    root = root || global.document;
    const radios = root.querySelectorAll('input[type=radio][name="' + name + '"]');
    if (!radios.length) return;
    const saved = get(key);
    if (saved !== undefined && saved !== null) {
      for (const r of radios) {
        if (r.value === saved && !r.disabled) { r.checked = true; break; }
      }
    }
    const save = function () {
      for (const r of radios) {
        if (r.checked) { set(key, r.value); return; }
      }
    };
    for (const r of radios) r.addEventListener('change', save);
  }

  /**
   * Remember a single checkbox. On call: applies the saved checked-state if one
   * was stored, then saves the boolean on every `change`.
   */
  function rememberCheckbox(el, key) {
    if (!el) return;
    const saved = get(key);
    if (typeof saved === 'boolean') el.checked = saved;
    el.addEventListener('change', function () { set(key, el.checked); });
  }

  global.DeepScryPrefs = {
    get, set, KEYS, STORE_KEY,
    rememberTextInput, rememberSelect, rememberRadioGroup, rememberCheckbox,
  };
})(typeof window !== 'undefined' ? window : globalThis);
