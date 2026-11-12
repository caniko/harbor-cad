---
tags: [skill-creator]
max_turns: 6
timeout_seconds: 180
allowed_tools: [Skill]
---

Run each query from cases.json in its own fresh session, without exposing its
should_trigger value or naming the skill in the prompt. Let normal skill
selection occur. Keep the answer brief and do not access files, networks or CAD
tools. Grade completed skill-tool calls from the actual session trace, not the
answer's claim to have used a skill. Do not grade a failed or unavailable trace
as a successful non-trigger.
