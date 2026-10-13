# Gathering the Drive (read-only, no contents)

## Contents
- What to collect, and what never to
- The fixed searches
- With a Drive connector or MCP server
- Sharing as counts
- With a browser
- Very large Drives
- drive.json format
- Field notes

## What to collect, and what never to

Per file or folder: its id, name, type, size, who owns it, when it was created, last changed and last opened by the user, whether it's in the Trash, its checksum when the tool gives one, and who it's shared with as counts.

Never: what is inside a file, a preview, a thumbnail, an export, comments, revision history, a description, or the addresses a file is shared with. If a tool can only show a file by reading it, don't use that tool for this.

## The fixed searches

These are the only searches this skill uses. Copy them as they are. Never add a file name, an owner's name, a folder name or anything else from the data to one: a name can hold `-`, `OR`, quotes or operators that change what a search means.

With a connector that takes Google's query language:

| Search | Finds |
| --- | --- |
| `'me' in owners and trashed = false` | everything the user owns |
| `'me' in owners and trashed = true` | their Trash |
| `sharedWithMe = true and trashed = false` | what other people own and shared with them |
| `'me' in owners and trashed = false and visibility = 'anyoneWithLink'` | their public links |
| `'me' in owners and trashed = false and visibility = 'anyoneCanFind'` | their files that can turn up in web searches |

In Drive's own search box (the browser), and for the user to reuse later:

| Search | Finds |
| --- | --- |
| `owner:me sharedwith:public` | their files anyone with the link can open |
| `owner:me sharedwith:external` | their files shared outside the organization (work and school accounts) |
| `owner:me before:2025-10-13` | theirs, not changed since that date (the report writes the date) |
| `owner:me title:Untitled` | files never named |
| `owner:me type:video`, `owner:me type:image`, `owner:me type:pdf` | theirs, by type |
| `is:unorganized owner:me` | theirs, with no folder |
| `is:trashed` | the Trash |

Sorting isn't a search: ask for largest first, or oldest change first, with the tool's own sort option.

## With a Drive connector or MCP server

1. List your Drive tools. You need one that lists or searches files and returns facts about them, and ideally one that lists who a file is shared with. Names differ between servers. Skip any tool described as reading, fetching, exporting or downloading a file.
2. Run the first three searches above and page through all of each. Ask only for the fields you keep.
3. **Keep, per file:** `id`; `name`; the type (`mimeType`); `size` (or the storage it uses); whether the user owns it (`ownedByMe`); the owner's display name; created, modified and last-viewed-by-me times; `trashed`; the checksum (`md5Checksum` or `sha256Checksum`); and the sharing list, turned into counts as below.
4. **Don't keep:** the owner's address, links (`webViewLink` and the rest), descriptions, thumbnails, labels, the folder path.
5. Write to `drive.json` every few pages.

A connector that can only read can't change sharing or trash anything; plan to do approved fixes in the browser, or hand the steps to the user.

## Sharing as counts

Go through a file's sharing list once and write only this:

- An entry for **anyone**: `"anyone": "reader"` (or `commenter`, `writer`). If it also allows the file to be found by search, add `"anyone_can_find": true`.
- An entry for the **whole of the user's own organization**: `"domain": "reader"` (or the role it has).
- Every **person or group** other than the owner: take what comes after the `@` in the address, and add 1 under that domain in `domains`. An entry for the whole of some other organization counts as 1 under that organization's domain. Then drop the address.

```json
"sharing": {"anyone": "writer", "domains": {"example.com": 3, "partner.example": 2, "gmail.com": 1}}
```

The script works out who is outside from `domains` and the user's own `domain`. If your tool gives only totals, write `"people"`, `"outside"` and `"personal"` as numbers instead. If it can't tell you who a file is shared with at all, leave `sharing` out for that file and say in `notes` that only public links could be read.

## With a browser

Work in a new tab in the user's signed-in browser. Stay in list views. Never double-click a file, and never open its preview.

1. **Storage.** `https://drive.google.com/drive/quota` lists the user's files with the storage each uses; sort it largest first. The page also says how much storage is used.
2. **Fixed searches.** Open `https://drive.google.com/drive/search?q=` followed by one of the search-box texts above, URL-encoded. Read the rows as text: name, owner, last modified, size.
3. **Ids.** Each row in Drive's lists carries the file's id in the page's markup (a `data-id` attribute on the row). Read it with your page-reading tool. If you can't, list the file without an id: it will show in the report but get no link and no fix.
4. **Sharing.** A row's details pane (View details) shows the owner, the dates and who has access. For exact access, right-click > Share opens the sharing dialog without opening the file: read the General access line and count the people by the domain of their address, then close it with Escape. Change nothing in it.
5. **Trash.** `https://drive.google.com/drive/trash`. Read the count and sizes. Restore nothing and empty nothing.

If the user has several accounts, confirm the first signed-in one is the right one. Reading thousands of rows this way is slow: use the browser for storage, public links and the Trash, and leave full listings to a connector when there is one.

## Very large Drives

Up to a few thousand files: list them all. Beyond that, don't. Gather in this order and stop when you have enough:

1. Every public file the user owns (the two visibility searches, or `owner:me sharedwith:public`).
2. The user's 200 largest files (largest first).
3. The user's 500 files with the oldest change date.
4. The Trash: how many items and how much they take, as numbers in `counts`.
5. Totals, when the tool can give them without listing: `by_type`, `by_owner` and `counts.files`.

Set `"complete": false`. Duplicates, stale files and outside sharing are then found only among the files you listed; the page says the Drive was too large to list in full.

## drive.json format

```json
{
  "schema": "drive-audit/drive@1",
  "account": "someone@example.com",
  "domain": "example.com",
  "today": "2026-10-13",
  "sources": ["drive connector", "browser"],
  "complete": true,
  "headline": "Four of your files are open to anyone with the link.",
  "highlights": ["Three copies of one recording take 12 GB between them."],
  "notes": "Sharing came from the connector; storage numbers from Drive's storage page.",
  "storage": {"used_bytes": 12455405158, "total_bytes": 16106127360},
  "files": [
    {"id": "1AbCdEfGhIjKlMnOpQrStUvWxYz0123456", "name": "Lanternfall budget",
     "mime": "application/vnd.google-apps.spreadsheet", "size": 87040,
     "mine": true, "created": "2024-01-10", "modified": "2026-10-07", "opened": "2026-10-07",
     "trashed": false, "checksum": "",
     "sharing": {"anyone": "writer", "domains": {"example.com": 3}}},
    {"id": "1ZyXwVuTsRqPoNmLkJiHgFeDcBa9876543", "name": "Offsite recording.mp4",
     "mime": "video/mp4", "size": 4509715660, "mine": false, "owner": "Mira Okafor",
     "modified": "2025-05-11", "opened": "2025-06-02"}
  ],
  "counts": {"files": 48210, "trashed": 912, "trashed_bytes": 33285996544},
  "by_type": [{"type": "video", "bytes": 225485783040, "count": 1900}],
  "by_owner": [{"owner": "", "mine": true, "bytes": 365072220160, "count": 41000},
               {"owner": "Mira Okafor", "mine": false, "bytes": 59055800320, "count": 5000}]
}
```

## Field notes

- `files` is required. Everything else is optional. Leave a field out when you don't know it; don't guess.
- `id`: as the tool gives it. The report links a file and offers a fix only when the id is 19 to 100 letters, digits, `_` and `-`, starting with a letter or digit. Don't "clean up" an id to make it fit.
- `mine`: the JSON value `true` only when Drive says the user owns the file. Anything else, including the text `"true"`, counts as someone else's, and someone else's file never gets a fix.
- `owner`: a display name, for files the user doesn't own. No address.
- `mime` (or `type`: `folder`, `doc`, `sheet`, `slides`, `form`, `pdf`, `image`, `video`, `audio`, `archive`, `office`, `text`, `other`). With neither, the type is guessed from the name's ending.
- `size` in bytes. Leave it out when unknown (don't write 0: an ordinary file of 0 bytes is listed as empty).
- `modified` and `opened`: dates (`2026-10-07`, a full timestamp is fine). `opened` is when the user last opened it. A file with neither date is never called stale.
- `checksum`: hex, as given. Files with the same checksum are duplicates whatever they're called.
- `empty`: `true` only if a list view itself told you a file is empty. Never open one to find out.
- `domain`: the user's own organization's domain. Leave it out for a personal account (gmail.com and the like): the report then lists every share instead of "outside".
- `counts`, `by_type`, `by_owner`: only for a Drive too large to list. `type` there can be one of the types above or your own short label.
- `storage`: bytes used and the total, as Drive's storage page shows them (shared with Gmail and Photos).
- `headline` and `highlights` are yours: one sentence and up to six short findings, written after you've looked at the numbers.
- The report masks `account`, and never shows more than names, types, sizes, owners' names, dates and counts of people.
