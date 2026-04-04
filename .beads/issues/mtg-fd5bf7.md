---
title: Implement ExtraTurn effect (Time Walk)
status: closed
priority: 2
issue_type: task
created_at: 2026-04-04T12:23:11.416588882+00:00
updated_at: 2026-04-04T12:23:18.641146598+00:00
closed_at: 2026-04-04T12:23:18.641146508+00:00
---

# Description

Added ApiType::AddTurn, Effect::ExtraTurn variant, extra_turns VecDeque on GameState, and turn consumption in advance_step(). Time Walk previously resolved as a silent no-op because AddTurn wasn't in the ApiType enum.
