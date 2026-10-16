# Gathering the mailbox (read-only)

## Contents
- The window and what to count first
- With a Gmail connector or MCP server
- With a browser
- Large mailboxes
- mailbox.json format
- Gmail search operators

## The window and what to count first

The window is the last 60 days: `newer_than:60d` (or `after:2026/08/14 before:2026/10/14`). Get these counts before listing anything. They size the job and go into `counts`:

| Search | Stored under |
| --- | --- |
| `in:inbox newer_than:60d` | `in:inbox` |
| `newer_than:60d is:unread` | `is:unread` |
| `category:promotions is:unread older_than:14d` | same text |
| `category:social older_than:30d` | same text |
| `larger:10M` | `larger:10M` |
| `in:spam` | `in:spam` |

Spam and Trash aren't in normal search results. Use `in:spam` to look there (an API tool may need an "include spam and trash" option).

## With a Gmail connector or MCP server

1. List your Gmail tools. You need a search or list tool and, ideally, one that returns headers. Names differ between servers.
2. Search `newer_than:60d` and page through the results. For each message keep only: id, date and time, From, subject, label ids, size, attachment names and sizes, and whether the user replied (a `SENT` message in the same thread) when the tool shows threads. If the tool returns headers, also keep whether `List-Unsubscribe` exists, whether `List-Unsubscribe-Post: List-Unsubscribe=One-Click` exists, `Reply-To`, and the result in `Authentication-Results` (pass or fail).
   Keep the time as the tool gives it, with its offset: ISO 8601 (`2026-10-01T08:12:00Z` or `...+03:00`), the `Date` header (`Wed, 01 Oct 2026 08:12:00 +0000`) or Gmail's `internalDate` (milliseconds) all work. The page shows when mail arrives on the user's clock, so also put the user's own offset in `utc_offset` (from the PC's clock, for example `"+03:00"`). A date without a time still counts, it just stays out of the weekday-by-hour map.
3. Don't fetch bodies. If a tool only offers "read the whole message", use it for the 20 biggest senders (one message each, for the headers) and take the rest from search results.
4. Label ids to expect: `INBOX`, `UNREAD`, `STARRED`, `IMPORTANT`, `SPAM`, `TRASH`, `SENT`, and `CATEGORY_PERSONAL`, `CATEGORY_PROMOTIONS`, `CATEGORY_SOCIAL`, `CATEGORY_UPDATES`, `CATEGORY_FORUMS`.
5. Write to `mailbox.json` every few pages.

Reading through the API leaves mail unread. A connector that can only read and draft can't unsubscribe, filter or archive; plan to do approved actions in the browser or hand the steps to the user.

## With a browser

Work in a new tab in the user's signed-in browser. Read pages; don't click on emails.

1. **Counts.** Open `https://mail.google.com/mail/u/0/#search/` followed by the URL-encoded search. Gmail shows "1-50 of N" at the top right. If it says "many", page forward once or narrow the dates. If the user has several accounts, confirm `/u/0/` is the right one.
2. **Tabs.** Search `category:promotions newer_than:60d`, then `social`, `updates`, `forums`, `primary`. Read the list as text. Each row gives sender, subject, date, and whether it's unread (bold, or "unread" in the row's accessible name).
3. **Senders with Unsubscribe.** If the left menu has **Manage subscriptions** (under More), open it. It lists senders, roughly how often they write, and an Unsubscribe button each. Record those addresses in `gmail_unsubscribe_offered`. Don't press the buttons.
4. **Spam.** Search `in:spam`. Record the count and a sample of senders and subjects from the list. Don't open anything there.
5. **Storage.** The bottom of the inbox shows "x GB of y GB used". Put it in `storage`.

Paging through thousands of rows is slow. Use the browser for counts, tabs, Manage subscriptions, Spam and storage, and leave bulk listing to a connector when there is one.

## Large mailboxes

- Up to about 2,000 messages in the window: list them all (`messages`).
- More: don't list everything. Find the top senders (page the first 500 to 1,000 results, or use Manage subscriptions), then count per sender with `from:<address> newer_than:60d` and `from:<address> newer_than:60d is:unread`. Do that only for plain addresses (letters, digits and `. _ + -`, starting with a letter or digit). Gmail reads a leading `-` as "not", so skip any other address, note it in `notes`, and let the report flag it. Put those in `senders` and set `"complete": false`.
- You can mix both: `messages` for what you listed, `senders` for the ones you only counted. A sender in both is taken from `messages`.
- For the chart of mail over time, add `daily`: Gmail's count per day (`after:2026/10/01 before:2026/10/02`), as `{"2026-10-01": 41}`. Without listed messages, that chart draws all mail in one colour.

## mailbox.json format

```json
{
  "schema": "inbox-cleanup/mailbox@1",
  "account": "someone@example.com",
  "window": {"start": "2026-08-14", "end": "2026-10-13"},
  "utc_offset": "+03:00",
  "sources": ["gmail connector", "browser"],
  "complete": true,
  "headline": "Most of your mail is from shops, and the people you write to still get through.",
  "insights": [
    {"title": "Mornings belong to newsletters", "text": "Example Digest lands before 7:00 most days and 9 in 10 stay unread. One unsubscribe takes 51 emails a month off your mornings."},
    {"title": "The people you answer", "text": "Mail from people is 1 in 6 emails and you open nearly all of it. Sam and Ana are the 2 you reply to most."}
  ],
  "sender_notes": {"weekly@samplenews.example": "Opened on Sunday mornings, so it's worth keeping, out of the inbox."},
  "notes": "Spam folder was read in the browser; headers came from the connector.",
  "counts": {"in:inbox": 1840, "category:promotions is:unread older_than:14d": 412, "larger:10M": 2, "in:spam": 37},
  "storage": {"used_gb": 7.4, "total_gb": 15},
  "gmail_unsubscribe_offered": ["hello@demo-fitness.example"],
  "messages": [
    {"id": "m1", "date": "2026-10-01T08:12:00Z", "from": "Example Store <news@example-store.example>",
     "subject": "Up to 40% off this weekend", "labels": ["INBOX", "UNREAD", "CATEGORY_PROMOTIONS"],
     "size": 48211, "list_unsubscribe": true, "one_click": true,
     "attachments": [{"name": "catalogue.pdf", "size": 1200000}],
     "reply_to": "", "auth": "pass", "replied": false}
  ],
  "senders": [
    {"email": "weekly@samplenews.example", "name": "Sample News Weekly", "count": 9, "unread": 4,
     "category": "updates", "list_unsubscribe": true, "last_date": "2026-10-10", "sample_subject": "Your weekly summary"}
  ],
  "daily": {"2026-10-01": 41, "2026-10-02": 37}
}
```

- `messages` or `senders` is required. Everything else is optional.
- `from` can be `Name <address>`, or give `from_name` and `from_email`.
- Unread comes from the `UNREAD` label or `"unread": true`. The category comes from a `CATEGORY_*` label or `"category": "promotions"`. Spam from the `SPAM` label or `"spam": true`. Starred from the `STARRED` label or `"starred": true`.
- `auth` is `pass`, `fail` or `none`. Leave fields out when you don't know them. Don't guess.
- `utc_offset` is the user's own offset from UTC (`"+03:00"`, `"-05:00"`). Message times that carry their own offset are moved to it. Leave it out and times are shown as written.
- `headline`, `insights` and `sender_notes` are yours, written after you've looked at the numbers: one sentence; 3 to 6 findings as `{"title", "text"}`; and a line for a sender where it helps. SKILL.md says what makes a good one. `highlights` (plain sentences, from older copies of this skill) are shown when there are no `insights`.
- Everything you write is shown as text, never run, and cut short if it's very long (headline 220 characters, insight text 420, a note 160).
- The report masks `account` and never shows more than sender, address, subject line, dates, sizes and counts.

## Gmail search operators

| Operator | Meaning |
| --- | --- |
| `newer_than:60d`, `older_than:1y` | relative age (d, m, y) |
| `after:2026/08/14`, `before:2026/10/14` | absolute dates |
| `category:promotions` (`primary`, `social`, `updates`, `forums`) | inbox tab |
| `is:unread`, `is:read`, `is:starred`, `is:important` | state |
| `in:inbox`, `in:spam`, `in:trash`, `in:anywhere` | where |
| `from:name@example.com`, `from:(a@x.example OR b@y.example)` | sender |
| `has:attachment`, `larger:10M`, `smaller:1M`, `filename:pdf` | attachments and size |
| `label:name`, `has:nouserlabels` | labels |
| `-` before a term, `OR`, `( )` | exclude, either, group |
