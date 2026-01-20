---
title: Card ID 0 appears in game logs
status: open
priority: 3
issue_type: bug
created_at: 2026-01-20T10:19:24.519133871+00:00
updated_at: 2026-01-20T10:19:24.519133871+00:00
---

# Description

## Bug Description

Cards occasionally appear in game logs with ID 0, which is a placeholder/invalid ID.

## Evidence

From game logs:
```
[GAMELOG Turn9 M1] All Hallow's Eve (42) puts 2 Scream counter(s) on Bazaar of Baghdad (0)
[GAMELOG Turn17 M1] Random1 plays City of Brass (0)
```

In both cases, the card name is correct but the ID is 0 instead of the actual CardId.

## Root Cause

Unknown - needs investigation. Possible causes:
1. Counter placement targeting uses unresolved placeholder
2. Land play logging uses wrong card reference
3. Some code path creates CardId(0) as placeholder and doesn't resolve it

## Impact

- Confusing logs make debugging difficult
- May indicate deeper targeting bugs where wrong permanents are affected
- All Hallow's Eve case suggests counters may be placed on wrong permanent

## Related

This may be related to the SearchLibrary placeholder bug (mtg-74577) - both involve unresolved placeholders.
