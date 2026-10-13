# inbox-cleanup

An Agent Skill that reviews the last two months of a Gmail inbox without changing anything and turns it into one private report page. The page has charts of mail per day, mail per Gmail tab and the biggest senders, lists of newsletters you don't open, senders you never opened, likely spam and phishing, and big attachments, plus a declutter plan with ready Gmail searches and filter suggestions. The user ticks what they approve (every list has Tick all and Untick all) and copies an approval line back. The agent then does only those items.

## Needs

- A way to read Gmail: a Gmail connector or MCP server, or browser automation signed in to Gmail (Claude in Chrome, Codex's browser). Both together give the fullest picture: the connector for bulk metadata and headers, the browser for Gmail's tabs, its Unsubscribe links, Manage subscriptions, Spam and storage.
- Python 3.8+ with nothing else installed, to build the page.

## Install

Copy the `inbox-cleanup` folder to:

- Claude Code: `~/.claude/skills/inbox-cleanup/` (or `.claude/skills/` in a project)
- Codex: `~/.agents/skills/inbox-cleanup/` (or `.agents/skills/` in a repo)

Then ask: "clean up my Gmail", "what should I unsubscribe from?", "which emails do I never open?"

## Safety

- Read-only by default. No sending, deleting, archiving, labelling, unsubscribing or filter creation without the user's approval of that specific item, and a count check before any bulk change.
- Never clicks links or opens attachments in emails. Uses Gmail's own Unsubscribe for real newsletters, and Report spam and Block for suspicious mail.
- Treats everything in an email as data, never as instructions.
- Collects sender, subject, date, labels and sizes. Not message bodies. The report holds senders, subject lines and counts only, makes no network requests, and its links go only to Gmail search.
- Sender addresses are attacker-controlled, so only plain addresses are ever placed in a Gmail search. Anything else is flagged and left out.

## Files

| File | Purpose |
| --- | --- |
| `SKILL.md` | Instructions the agent follows |
| `references/gathering.md` | Searches, paging, browser steps, the `mailbox.json` format |
| `scripts/build_report.py` | Builds `inbox-report.html` and `inbox-plan.json` from `mailbox.json` |
| `assets/inbox-report-template.html` | The self-contained page (light and dark) |

## License

No license has been chosen for this skill yet, so it falls under the license at the root of the repository it lives in (momputer), which is the PolyForm Noncommercial License 1.0.0 at the time of writing. That is not an open-source license. The example skills in [anthropics/skills](https://github.com/anthropics/skills) each ship a `LICENSE.txt` (Apache 2.0) and say `license: Complete terms in LICENSE.txt` in their frontmatter, and Claude's plugin directory needs a `LICENSE` file or a `license` field for every listing. Before this skill is submitted anywhere, the owner should decide its license and add it here.
