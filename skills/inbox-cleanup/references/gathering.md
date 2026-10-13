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
2. Search `newer_than:60d` and page through the results. For each message keep only: id, date, From, subject, label ids, size, attachment names and sizes. If the tool returns headers, also keep whether `List-Unsubscribe` exists, whether `List-Unsubscribe-Post: List-Unsubscribe=One-Click` exists, `Reply-To`, and the result in `Authentication-Results` (pass or fail).
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
- More: don't list everything. Find the top senders (page the first 500 to 1,000 results, or use Manage subscriptions), then count per sender with `from:<address> newer_than:60d` and `from:<address> newer_than:60d is:unread`. Put those in `senders` and set `"complete": false`.
- You can mix both: `messages` for what you listed, `senders` for the ones you only counted. A sender in both is taken from `messages`.

## mailbox.json format

```json
{
  "schema": "inbox-cleanup/mailbox@1",
  "account": "someone@example.com",
  "window": {"start": "2026-08-14", "end": "2026-10-13"},
  "sources": ["gmail connector", "browser"],
  "complete": true,
  "headline": "Half of this inbox is promotions that were never opened.",
  "highlights": ["Six shops send about 30 emails a week; almost none are opened."],
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
  ]
}
```

- `messages` or `senders` is required. Everything else is optional.
- `from` can be `Name <address>`, or give `from_name` and `from_email`.
- Unread comes from the `UNREAD` label or `"unread": true`. The category comes from a `CATEGORY_*` label or `"category": "promotions"`. Spam from the `SPAM` label or `"spam": true`.
- `auth` is `pass`, `fail` or `none`. Leave fields out when you don't know them. Don't guess.
- `headline` and `highlights` are yours: one sentence and up to six short findings, written after you've looked at the numbers.
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
