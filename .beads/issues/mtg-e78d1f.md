---
title: 'Bugfix: Bazaar of Baghdad SubAbility chain in activated abilities'
status: closed
priority: 2
issue_type: task
created_at: 2026-04-04T12:23:11.414976666+00:00
updated_at: 2026-04-04T12:23:18.639619380+00:00
closed_at: 2026-04-04T12:23:18.639619280+00:00
---

# Description

Fixed parse_activated_abilities() to follow SubAbility$ chains. Previously only the primary effect was parsed, so Bazaar of Baghdad's 'Draw 2, then Discard 3' only produced DrawCards{2} without the DiscardCards{3}. Fix: Added self.follow_sub_ability_chain() after primary effect parsing in card.rs.
