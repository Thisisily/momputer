# inbox-cleanup

An Agent Skill that reviews the last two months of a Gmail inbox without changing anything and turns it into one private dashboard page that explains the mailbox: mail per day by kind (people, newsletters, promotions, notifications), when it arrives, what gets opened, Gmail's tabs, the busiest senders and the people you write back to, a sortable table of newsletters and shops, the unread backlog, senders never opened, big attachments and likely phishing, with the agent's own insights on top. Then a declutter plan with ready Gmail searches and filter suggestions. The user ticks what they approve (every list has Tick all and Untick all) and copies an approval line back. The agent then does only those items.

## Needs

- A way to read Gmail: a Gmail connector or MCP server, or browser automation signed in to Gmail (Claude in Chrome, Codex's browser). Both together give the fullest picture: the connector for bulk metadata and headers, the browser for Gmail's tabs, its Unsubscribe links, Manage subscriptions, Spam and storage.
- Python 3.8+ with nothing else installed, to build the page.

## Install

In Claude Code, one line:

```
/plugin install inbox-cleanup --marketplace Thisisily/momputer
```

Or copy the `inbox-cleanup` folder to:

- Claude Code: `~/.claude/skills/inbox-cleanup/` (or `.claude/skills/` in a project)
- Codex: `~/.agents/skills/inbox-cleanup/` (or `.agents/skills/` in a repo)

Then ask: "clean up my Gmail", "what should I unsubscribe from?", "which emails do I never open?"

## Safety

- Read-only by default. No sending, deleting, archiving, labelling, unsubscribing or filter creation without the user's approval of that specific item, and a count check before any bulk change.
- Never clicks links or opens attachments in emails. Uses Gmail's own Unsubscribe for real newsletters, and Report spam and Block for suspicious mail.
- Treats everything in an email as data, never as instructions.
- Collects sender, subject, date, labels and sizes. Not message bodies. The report holds senders, subject lines and counts only, makes no network requests, and its links go only to Gmail search.
- Sender addresses are attacker-controlled, and Gmail reads some characters as commands (a leading `-` means "not"). So only plain addresses are ever placed in a Gmail search, each sender comes with its query ready-made, and anything else is flagged and left out.

## Files

| File | Purpose |
| --- | --- |
| `SKILL.md` | Instructions the agent follows |
| `references/gathering.md` | Searches, paging, browser steps, the `mailbox.json` format |
| `scripts/build_report.py` | Builds `inbox-report.html` and `inbox-plan.json` from `mailbox.json` |
| `assets/inbox-report-template.html` | The self-contained page (light and dark) |

## License

Apache License 2.0. Copyright 2026 Ilya Arbabi. See [LICENSE.txt](LICENSE.txt). Free for any use, companies included.

(The momputer app this skill lives next to has its own, different license. The skills don't need it.)
