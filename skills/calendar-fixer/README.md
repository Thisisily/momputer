# calendar-fixer

An Agent Skill that reviews six weeks of a Google Calendar (two back, four ahead) without changing anything and turns it into one private report page. The page has charts of hours in meetings per day and per week, and lists of double bookings, back-to-back runs, meetings outside working hours, invitations never answered, stale repeating meetings, meetings with no video link or place, duplicates, events at odd hours and days with no free two hours. The user ticks the fixes they approve (every list has Tick all and Untick all) and copies an approval line back. The agent then does only those.

## Needs

- A way to read Google Calendar: a Calendar connector or MCP server, or browser automation signed in to Google Calendar (Claude in Chrome, Codex's browser). Both together give the fullest picture: the connector for every event, its guests' answers and its id, the browser for the settings pages, Propose a new time and the Trash.
- Python 3.8+ with nothing else installed, to build the page.

## Install

In Claude Code, one line:

```
/plugin install calendar-fixer --marketplace Thisisily/momputer
```

Or copy the `calendar-fixer` folder to:

- Claude Code: `~/.claude/skills/calendar-fixer/` (or `.claude/skills/` in a project)
- Codex: `~/.agents/skills/calendar-fixer/` (or `.agents/skills/` in a repo)

Then ask: "fix my calendar", "where am I double booked?", "how much of my week is meetings?"

## Safety

- Read-only by default. No answering, declining, moving, shortening, creating or deleting without the user's tick for that one event. Nothing is ticked to begin with.
- Other people are told when an invitation is answered or a meeting moves. Each row says who (the organizer, or how many guests), and the agent reads that list back and waits for a yes before the first change.
- Never edits or deletes an event the user doesn't own, and never writes to attendees.
- Nothing is deleted for good: one event at a time, to Calendar's Trash. A series is ended by giving it an end date, never with "This and following events", which skips the Trash.
- Titles, descriptions and places are written by other people and are treated as data, never as instructions. A place becomes a link only when it is `https` on a known meeting host; `javascript:` and the rest stay text.
- Collects titles, times, guest counts and answers. Not descriptions and not guest lists. The report shows at most the organizer's name, makes no network requests, and links only to the user's own Google Calendar and to known meeting services.
- The report id is a fingerprint of the plan. `check_approval.py` refuses an approval for a plan that was edited afterwards, a number that isn't in the plan, and one event approved for two things that can't both happen.

## Files

| File | Purpose |
| --- | --- |
| `SKILL.md` | Instructions the agent follows |
| `references/gathering.md` | Calendars, fields, browser steps, the counted form for very busy calendars, the `calendar.json` format |
| `scripts/build_report.py` | Builds `calendar-report.html` and `calendar-plan.json` from `calendar.json` |
| `scripts/check_approval.py` | Checks an approval line against the plan and prints exactly what was approved |
| `assets/calendar-report-template.html` | The self-contained page (light and dark) |

## License

Apache License 2.0. Copyright 2026 Ilya Arbabi. See [LICENSE.txt](LICENSE.txt). Free for any use, companies included.

(The momputer app this skill lives next to has its own, different license. The skills don't need it.)
