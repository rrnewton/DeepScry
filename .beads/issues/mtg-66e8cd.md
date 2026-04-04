---
title: Replace silent no-ops with Effect::Unimplemented
status: closed
priority: 2
issue_type: task
created_at: 2026-04-04T12:23:11.420732652+00:00
updated_at: 2026-04-04T12:23:18.644459983+00:00
closed_at: 2026-04-04T12:23:18.644459903+00:00
---

# Description

Changed effect_converter.rs catch-all from '_ => None' (silently dropping unimplemented effects) to producing Effect::Unimplemented{api_type}. execute_effect() now logs a visible warning. Spells with unimplemented effects no longer silently resolve as no-ops.
