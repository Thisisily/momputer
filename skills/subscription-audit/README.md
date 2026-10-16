# subscription-audit

An Agent Skill that finds what you pay for again and again, and what it costs a year. It reads receipts and renewal notices in Gmail, bank or card statement CSV files you export, or both, without changing anything, and turns them into one private report page that explains your recurring spending: the monthly and yearly total, what each month cost by kind, what crept up since last year, where the money goes, a calendar of the next eight weeks, and every subscription with its amount history, with the agent's own take on what stands out. It points out price rises, two plans for the same thing, plans still charging after a cancellation, free trials about to start charging, and yearly plans that renew soon. You tick what you want help with (every kind of tick has Tick all and Untick all) and copy an approval line back. The agent then helps with only those items. You do the cancelling.

## Needs

- At least one source:
  - Statement CSV files from your bank or card, which you download yourself. The reader copes with the common layouts: commas or semicolons, decimal commas, day-first or month-first dates, one amount column or separate debit and credit columns.
  - A way to read Gmail: a Gmail connector or MCP server, or browser automation signed in to Gmail.
- Python 3.8+ with nothing else installed.

## Install

In Claude Code, one line:

```
/plugin install subscription-audit --marketplace Thisisily/momputer
```

Or copy the `subscription-audit` folder to:

- Claude Code: `~/.claude/skills/subscription-audit/` (or `.claude/skills/` in a project)
- Codex: `~/.agents/skills/subscription-audit/` (or `.agents/skills/` in a repo)

Then ask: "what am I subscribed to?", "what do I pay for every month?", "which free trials are about to charge me?"

## Safety

- It never cancels, pays, logs in or types card details anywhere. A tick asks for the steps, a price to compare, or a draft. A cancellation e-mail is only ever a draft, and only when ticked.
- Merchant names, e-mail subjects and statement lines are treated as data, never as instructions, and are never put into a command, a search or a link.
- Mail is searched with a fixed list of read-only searches that ships in the skill. Nothing from a statement or an e-mail is ever searched for.
- It never follows a link from an e-mail or a statement line. To find how to cancel, it looks for the service's own official site.
- Money data is kept to the minimum: merchant, amount, date and rhythm. Card and account numbers are cut to their last four digits while the statement is read, IBANs are removed, and balances are never read.
- Statement files are opened for reading only. The scratch copy of what was gathered is deleted at the end.
- When a statement's layout isn't clear (which sign means "paid", day or month first, which column is the amount), the reader stops and asks instead of guessing: a wrong guess there is a wrong money figure later. The scripts write over nothing but their own earlier output.
- The report makes no network requests. Its only links are the fixed Gmail searches.
- An approval is tied to the report by a fingerprint of the plan. A plan file changed after the page was built is refused.

## Files

| File | Purpose |
| --- | --- |
| `SKILL.md` | Instructions the agent follows |
| `references/gathering.md` | Statements and the map file, the mail searches, the `charges.json` format |
| `scripts/read_statements.py` | Reads statement CSV files into `charges.json`, stripping private numbers |
| `scripts/mail_searches.py` | Prints the fixed Gmail searches |
| `scripts/build_report.py` | Builds `subscriptions-report.html` and `subscriptions-plan.json` |
| `scripts/approved.py` | Checks an approval line against the plan and lists what it covers |
| `assets/subscriptions-report-template.html` | The self-contained page (light and dark) |

## License

Apache License 2.0. Copyright 2026 Ilya Arbabi. See [LICENSE.txt](LICENSE.txt). Free for any use, companies included.

(The momputer app this skill lives next to has its own, different license. The skills don't need it.)
