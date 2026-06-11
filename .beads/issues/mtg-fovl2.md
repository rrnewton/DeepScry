---
title: 'Cloud decks: stable server-side direct-link route (presigned redirect) for deck collection'
status: open
priority: 3
issue_type: task
created_at: 2026-06-11T13:19:14.739124290+00:00
updated_at: 2026-06-11T13:19:14.739124290+00:00
---

# Description

FOLLOW-UP (deck-editor UX batch2 item 11, option b): add a STABLE server-side direct-link route for a user's cloud deck collection.

CONTEXT: R2 deck-collection objects are PRIVATE — served only via short-TTL presigned SigV4 URLs (GET /api/deck-storage/credentials mints get_url/put_url/download_url). A permanent public link would require making the bucket public, which is a security risk (every user's decks become world-readable). 

SHIPPED NOW (option a): the deck editor's "Direct link" button (web/deck_editor.html + DeckStorage.directLink() in web/deck_storage.js) copies the CURRENT presigned GET URL to the clipboard with a clear "(temporary link, expires)" note. Works but the link lapses after the presign TTL.

WANTED (option b, this bead): a STABLE app route — e.g. GET /api/deck-storage/link (or /decks/me) — that, on each load, authenticates the session, re-mints a fresh presigned GET, and 302-redirects to it. The user can then bookmark ONE durable URL that always resolves to their latest collection without exposing the bucket. Auth = the same session cookie /auth/status uses; 401 when logged out. Keep the existing presigned-credentials + download paths working unchanged.

SCOPE: server-side (Rust web server route + the existing presign helper) + swap the editor's "Direct link" button to point at the stable route once it exists. Out of scope for the web-only batch2 pass (no server work), hence option (a) shipped first.
