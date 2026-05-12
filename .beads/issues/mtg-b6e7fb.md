---
title: 'Research: Find agentplay files and understand web GUI architecture'
status: closed
priority: 0
issue_type: task
created_at: 2026-04-04T01:50:09.418779162+00:00
updated_at: 2026-05-12T13:58:40.952306099+00:00
closed_at: 2026-05-12T13:58:40.952306019+00:00
---

# Description

Files: Find agentplay/*.py files (created by sub-orc, location TBD)

Action: 
1. Find where the agentplay files were committed (git log, find)
2. Read agent_game.py to understand current architecture
3. Find the web GUI entry point (WASM build, HTML, server setup)
4. Understand how the MTG engine exposes game state to the web GUI
5. Check if Playwright is available or needs to be installed

Why: Need to understand current state before extending for web GUI testing.

Verify: Report back with file locations, architecture summary, and feasibility assessment for Playwright integration.
