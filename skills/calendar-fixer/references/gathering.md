# Gathering the calendar (read-only)

## Contents
- The window, working hours and time
- With a Calendar connector or MCP server
- With a browser
- Very busy calendars: the counted form
- calendar.json format
- Field notes

## The window, working hours and time

The window is 14 days back and 28 days ahead of today, in the user's own time zone. The past two weeks are for patterns only (the charts, and how the user answered repeating meetings). Fixes are offered only from today on.

- **Working hours.** Read them from Calendar's settings when your tool shows them (Settings > General > Working hours; work and school accounts only). Otherwise ask. With nothing given the report assumes 09:00 to 18:00, Monday to Friday, and says so on the page.
- **Days off.** Ask about leave and public holidays in the next four weeks and list the dates in `days_off`. All-day Out of office events are picked up by themselves.
- **Time.** Write every time as the user's calendar shows it: `2026-10-14T10:00`. An offset at the end (`+02:00`) is accepted and ignored. Most Calendar tools take a time-zone parameter: pass the calendar's own zone. If your tool gives UTC (`...Z`), convert before writing; the script reads the clock time as written and prints a note when it sees `Z`.

## With a Calendar connector or MCP server

1. List your Calendar tools. You need one that lists calendars and one that lists events in a date range. Names differ between servers.
2. **Calendars.** Keep the ones the user owns: their primary calendar and any other they created (owner access). Give each a short key of your own (`work`, `personal`): letters, digits, `_` and `-`. Colleagues' calendars, team calendars the user only reads, holidays and birthdays aren't the user's time: leave them out, or list them with `"mine": false` and they are ignored.
3. **Events.** For each of those calendars, list the window with repeating events expanded into single events (in Google's API, `singleEvents=true`). Page through all of it.
4. **Keep, per event:** id; calendar key; title; start and end; whether it's all-day; whether the user is the organizer (`organizer.self`); the organizer's display name; the user's own answer (the guest entry marked `self`); how many guests there are and how many answered each way; whether it repeats and the id of its series (`recurringEventId`); the place; the video address (`hangoutLink`, or the conference entry point of type video); the kind (`eventType`); whether it's marked Free; whether it's cancelled; its time zone.
5. **Don't keep:** the description, guests' names or addresses, attachments, the creator's address, any other link. Count the guest list and drop it. Don't fetch a description to "understand" a meeting.
6. **Series facts.** For each series id you saw, read the series itself once (the event whose id is the series id) for the date it first happened and whether its repeat rule has an end (`UNTIL` or `COUNT`). Write a `series` row with `started` and `ends`. If the tool lets you look further back cheaply, count how the user answered the last 8 times and add `recent`.
7. Write to `calendar.json` every few pages.

A connector that can only read can't answer, move or delete; plan to do approved fixes in the browser, or hand the steps to the user.

## With a browser

Work in a new tab in the user's signed-in browser. Read; don't press buttons on events.

1. **Open the schedule.** `https://calendar.google.com/calendar/u/0/r/agenda/2026/10/13` lists events as text from that date (`/r/week/...` and `/r/day/...` show a week or a day). If the user has several accounts, confirm `/u/0/` is the right one. Make sure only the user's own calendars are switched on in the left panel, or note which rows belong to which calendar; don't switch calendars off yourself without saying so (it changes what they see).
2. **Read rows as text.** Each gives the time, the title and the calendar. Invitations not yet answered are drawn hollow (an outline with no fill), and the row's accessible name usually says an answer is needed.
3. **Guests and answers.** These are only in an event's details. Open one, read the guest summary ("6 guests, 3 yes, 1 no, 2 awaiting"), the organizer, the place and whether a video link is there (for an all-day event, also whether it shows as Busy or Free), then close it with Escape. Press nothing inside: not Yes, No or Maybe, not the pencil, not the bin, and no link in the description. For a busy calendar do this only for the events that matter (unanswered, overlapping, outside working hours, repeating).
4. **Settings (read only).** `https://calendar.google.com/calendar/u/0/r/settings` shows the time zone, working hours and whether speedy meetings is on.
5. **Ids.** The browser doesn't show event ids. Give each event your own (`b1`, `b2`, ...) and keep the day, time and title to find it again. For a repeating meeting use one made-up `series_id` for all its dates.

Reading four weeks event by event is slow. With a browser alone, use the counted form below.

## Very busy calendars: the counted form

When a calendar holds more than a few hundred events in the window, or the browser is your only source, count the days instead of listing everything:

- `days`: one row per day with how many meetings, how many minutes in meetings, the longest free stretch inside working hours and the longest run of meetings with no gap.
- `events`: only the ones that need a decision: unanswered invitations, anything you saw overlapping, anything outside working hours, duplicates, meetings with no link.
- `series`: one row per repeating meeting worth a look, with `next` (its next date), since its single dates aren't listed.
- `"complete": false`.

A date that has a `days` row uses those numbers for the charts, the free-block check and the back-to-back table. Events listed on that date are still checked for clashes, answers, links and the rest. Clashes between two events you didn't list can't be found; the page says so. No new time is suggested on a date that has a `days` row, or past the window's last day: the script can't see where those days are free. The plan then has no time for that meeting, and you ask the user for one.

## calendar.json format

```json
{
  "schema": "calendar-fixer/calendar@1",
  "account": "someone@example.com",
  "timezone": "Europe/Amsterdam",
  "today": "2026-10-13",
  "window": {"start": "2026-09-29", "end": "2026-11-09"},
  "working_hours": {"start": "09:00", "end": "18:00", "days": ["mon", "tue", "wed", "thu", "fri"]},
  "days_off": ["2026-10-23"],
  "sources": ["calendar connector", "browser"],
  "complete": true,
  "headline": "Tuesday the 20th is four meetings in a row, and five invitations are waiting.",
  "highlights": ["Two meetings sit on top of each other on Wednesday at 14:00."],
  "notes": "Guest answers came from the connector; working hours from the settings page.",
  "calendars": [
    {"key": "work", "name": "Work", "primary": true, "mine": true},
    {"key": "personal", "name": "Personal", "mine": true}
  ],
  "events": [
    {"id": "abc123_20261014T080000Z", "calendar": "work", "title": "Weekly sync",
     "start": "2026-10-14T10:00", "end": "2026-10-14T11:00",
     "mine": false, "organizer": "Mira Okafor", "my_response": "accepted",
     "attendees": {"total": 8, "accepted": 6, "declined": 1, "tentative": 0, "needs_action": 1},
     "recurring": true, "series_id": "abc123",
     "location": "Room Kestrel", "video": "https://meet.google.com/abc-defg-hij",
     "kind": "event", "busy": true, "status": "confirmed", "timezone": "Europe/Amsterdam"}
  ],
  "series": [
    {"id": "abc123", "started": "2022-03-07", "ends": "never",
     "recent": {"held": 8, "declined": 6, "accepted": 0}}
  ],
  "days": [
    {"date": "2026-10-20", "meetings": 11, "meeting_minutes": 420, "longest_free_minutes": 45,
     "longest_run_minutes": 210, "longest_run_meetings": 5}
  ]
}
```

A `series` row for a meeting whose dates you didn't list (the counted form) carries the rest too:

```json
{"id": "s-board", "title": "Board pre-read", "calendar": "work", "organizer": "Oskar Lindqvist", "mine": false,
 "started": "2020-06-01", "ends": "never", "my_response": "accepted",
 "attendees": {"total": 14, "accepted": 0, "declined": 3},
 "recent": {"held": 8, "declined": 6, "accepted": 0},
 "next": {"id": "s-board_20261019", "start": "2026-10-19T16:00", "end": "2026-10-19T17:00"}}
```

## Field notes

- `events` or `days` is required. Everything else is optional. Leave a field out when you don't know it; don't guess.
- `today`: the date where the user is. Add the time (`2026-10-13T14:30`) and meetings that already ended today get no fix.
- `id` and `series_id`: letters, digits, `_` and `-` only, starting with a letter or digit. Anything else is shown in the report but gets no fix. Don't "clean up" an id to make it fit.
- `calendar`: one of the keys in `calendars`. With no `calendars` list, everything is taken to be on the user's one calendar.
- `mine`: the JSON value `true` only when the user is the organizer. On a calendar, only when they own it. Anything else, including the text `"true"`, counts as no.
- `my_response`: `accepted`, `declined`, `tentative` or `needsAction`. Leave it out for an event with no guests.
- `attendees`: counts, the user included. `total` 0 or 1 means it's just them. Give the answer counts when you have them; without them the report won't say "nobody has accepted".
- All-day events: dates with no time (`"start": "2026-11-03", "end": "2026-11-04"`; the end is the day after, as Google gives it) or `"all_day": true`.
- `kind`: `event`, `focus`, `out_of_office`, `from_gmail` (a booking Calendar added from a mail), or `working_location`, `birthday`, `task` (those last three are ignored and never count as busy time).
- `busy`: `false` for an event marked Free (in Google's own data, `transparency` is `transparent`; the script reads that field too). Read it, don't leave it out: with nothing given an event counts as busy. A new time is never suggested on top of busy time the user hasn't declined, whatever its kind, and a busy all-day event keeps its whole day clear. Most all-day events are marked Free.
- `location` and `video` are text someone else typed. Pass them through as they are. The script shows a link only for `https` on meet.google.com, zoom.us, teams.microsoft.com, teams.live.com, webex.com and whereby.com (subdomains of zoom.us and webex.com included), and plain text for everything else.
- `series[].ends`: `"never"` when the repeat rule has no end, a date when it has one. Leave it out when you don't know.
- `headline` and `highlights` are yours: one sentence and up to six short findings, written after you've looked at the numbers.
- The report masks `account`, and never shows more than titles, times, calendar names, guest counts and organizers' names.
