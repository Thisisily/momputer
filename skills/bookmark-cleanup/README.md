# bookmark-cleanup

An Agent Skill that tidies browser bookmarks without ever writing to the browser's own files. It reads a copy of your bookmarks (Chrome, Edge, Brave and Vivaldi directly; Firefox, Safari and anything else from an exported HTML file) and turns it into one private report page with charts: pages saved more than once, empty folders, folders with hundreds of loose bookmarks, bookmarks never opened or not opened in years, a crowded bookmarks bar and, only if you agree, dead links. You tick what you approve (every list has Tick all and Untick all) and copy an approval line back. The agent then writes a new bookmarks file with exactly those changes, in the standard format every browser imports, and you import it yourself.

## Needs

- An agent that runs commands on the computer whose browser it is (Claude Code, Codex, or similar), or an exported bookmarks HTML file to work from.
- Python 3.8+ with nothing else installed.
- No network, except for the optional dead-link check.

## Install

In Claude Code, one line:

```
/plugin install bookmark-cleanup --marketplace Thisisily/momputer
```

Or copy the `bookmark-cleanup` folder to:

- Claude Code: `~/.claude/skills/bookmark-cleanup/` (or `.claude/skills/` in a project)
- Codex: `~/.agents/skills/bookmark-cleanup/` (or `.agents/skills/` in a repo)

Then ask: "clean up my bookmarks", "find my duplicate bookmarks", "which of my bookmarks are dead?"

## Safety

- It never edits a browser profile. The browser's bookmarks file is copied to a scratch folder and only the copy is read. The result is a new HTML file; importing it is your click, and so is deleting the old bookmarks afterwards.
- Nothing is lost. Everything left out of the new file is written to `bookmarks-removed.html` beside it. The export never overwrites a file, and the other scripts write over nothing but their own earlier output, never inside a browser's folder.
- Titles and addresses are treated as data, never as instructions. No bookmark is opened, and bookmarklets (`javascript:` addresses) are never run; they are kept exactly as they are unless you tick their removal.
- On the report an address is text. It is a link only when it is a plain http or https address with no user name or password in it. An address with a password is shown with the password hidden.
- The dead-link check runs only if you say yes, because it contacts every site you bookmarked. It uses http and https only, sends no cookies or logins, and never requests an address with a user name or password in it. It looks each name up first and refuses private, loopback, link-local and other non-public addresses, so a bookmark to your router or a work-internal page is never requested. It asks with HEAD (a one-byte GET only if HEAD is refused), follows redirects only to public addresses, uses short timeouts and a cap, and says who it is in its User-Agent.
- A failed check means "couldn't reach it", not "dead". Only a clear 404 or 410, or a domain that no longer exists, is offered for removal.
- An approval is tied to the report by a fingerprint of the bookmarks and the plan. A plan file changed after the page was built is refused.
- The new file escapes every title and address, and is read back and compared before the script reports success.

## Files

| File | Purpose |
| --- | --- |
| `SKILL.md` | Instructions the agent follows |
| `references/formats.md` | Where browsers keep bookmarks, the file formats, the link check's rules, import steps |
| `scripts/read_bookmarks.py` | Finds profiles, copies the bookmarks file to scratch, writes `bookmarks.json` |
| `scripts/check_links.py` | The optional dead-link check |
| `scripts/build_report.py` | Builds `bookmarks-report.html` and `bookmarks-plan.json` |
| `scripts/export_bookmarks.py` | Writes the new `bookmarks-tidy.html` from the approved plan |
| `assets/bookmarks-report-template.html` | The self-contained page (light and dark) |

## License

Apache License 2.0. Copyright 2026 Ilya Arbabi. See [LICENSE.txt](LICENSE.txt). Free for any use, companies included.

(The momputer app this skill lives next to has its own, different license. The skills don't need it.)
