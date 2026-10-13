# Bookmarks: where they live, the formats, importing

## Contents
- Where browsers keep bookmarks
- Getting an exported file (Firefox, Safari, any browser)
- What is read and what isn't
- bookmarks.json format
- The dead-link check in detail
- links.json format
- Importing the new file
- Replacing the old bookmarks instead of adding to them

## Where browsers keep bookmarks

Chrome, Edge, Brave, Vivaldi and Chromium keep them in a JSON file called `Bookmarks` (no extension) inside each profile folder. Profile folders are named `Default`, `Profile 1`, `Profile 2` and so on.

| | Windows | macOS | Linux |
| --- | --- | --- | --- |
| Chrome | `%LOCALAPPDATA%\Google\Chrome\User Data` | `~/Library/Application Support/Google/Chrome` | `~/.config/google-chrome` |
| Edge | `%LOCALAPPDATA%\Microsoft\Edge\User Data` | `~/Library/Application Support/Microsoft Edge` | `~/.config/microsoft-edge` |
| Brave | `%LOCALAPPDATA%\BraveSoftware\Brave-Browser\User Data` | `~/Library/Application Support/BraveSoftware/Brave-Browser` | `~/.config/BraveSoftware/Brave-Browser` |
| Vivaldi | `%LOCALAPPDATA%\Vivaldi\User Data` | `~/Library/Application Support/Vivaldi` | `~/.config/vivaldi` |

`read_bookmarks.py --list` looks in these places (and the usual Flatpak and Snap ones on Linux) and prints browser, profile folder, file size and date. Don't go looking through profile folders yourself, and never open `Bookmarks` in an editor. The script copies that one file to the scratch folder, reads the copy and deletes it. It skips a file or profile folder that is a link.

Firefox keeps bookmarks in a database (`places.sqlite`) and Safari in a property list. This skill doesn't read either; it reads the HTML file those browsers export.

## Getting an exported file (Firefox, Safari, any browser)

The user does this; it takes a few clicks. Menu names move a little between versions.

- **Firefox**: Bookmarks > Manage bookmarks (Ctrl+Shift+O) > Import and Backup > Export Bookmarks to HTML.
- **Safari**: File > Export > Bookmarks.
- **Chrome, Brave**: Bookmark manager (Ctrl+Shift+O) > the three dots at the top > Export bookmarks.
- **Edge**: Favorites (Ctrl+Shift+O) > the three dots > Export favorites.

Then: `python scripts/read_bookmarks.py --file "<that file>" --out "<out>/bookmarks.json"`.

An exported file doesn't say when a bookmark was last opened, so "not opened in years" and the bookmarks-bar list stay empty for it. For a Chromium browser on this computer, reading the profile directly gives the fuller report.

## What is read and what isn't

From a Chromium `Bookmarks` file: each folder and bookmark with its name, address, the date it was added and the date it was last opened (`date_last_used`; browsers began recording it around 2022, and 0 means not opened since). The three roots are the bookmarks bar, Other bookmarks (shown as the top level) and Mobile bookmarks (a folder of its own, last).

From an exported HTML file (the Netscape bookmark format): folders (`<H3>`), bookmarks (`<A HREF>`), `ADD_DATE`, Firefox `TAGS`, and the marks for the toolbar folder (`PERSONAL_TOOLBAR_FOLDER`) and Firefox's Other Bookmarks (`UNFILED_BOOKMARKS_FOLDER`).

Not carried into the new file: site icons (the browser fetches them again), descriptions (`<DD>`), separators (`<HR>`), Firefox keywords, and the last-opened dates (the exchange format has no field browsers read back). Tell the user if they use Firefox keywords or separators.

## bookmarks.json format

```json
{
  "schema": "bookmark-cleanup/bookmarks@1",
  "source": {"kind": "chromium", "browser": "edge", "profile": "Default", "has_last_used": true, "read": "2026-10-13T09:30:00"},
  "nodes": [
    {"id": 1, "type": "folder", "parent": 0, "title": "Bookmarks bar", "root": "bar"},
    {"id": 2, "type": "url", "parent": 1, "title": "Lanternfall", "url": "https://lanternfall.example/watch",
     "added": "2021-07-07", "added_ts": 1625659200, "last_used": "2026-10-10"},
    {"id": 3, "type": "folder", "parent": 0, "title": "Recipes", "added": "2022-01-05", "added_ts": 1641384000}
  ]
}
```

- `nodes` is in file order; `id` counts from 1; `parent` is a folder's id, or 0 for the top level. A parent always comes before its children.
- `root` is `bar` for the bookmarks bar and `unfiled` for Firefox's Other Bookmarks folder.
- `last_used` is missing when the bookmark was never opened or the source doesn't record it (`has_last_used` says which).
- Titles and addresses are stored exactly as found. They are other people's text: don't read this file into the conversation.

## The dead-link check in detail

`check_links.py` asks each distinct address once. What it will and won't do is fixed in the script:

| Rule | Why |
| --- | --- |
| http and https only | Nothing else is a web page. `javascript:`, `file:`, `data:` are never touched. |
| No cookies, no saved logins, no proxy settings | It must look like a stranger knocking, not like the user. |
| An address with a user name or password in it is never requested | That would send the credentials. |
| The name is looked up first; if any address it points to is private, loopback, link-local, carrier-grade NAT, documentation, multicast or otherwise not public, nothing is requested | A bookmark to a router, a printer, a NAS or a work-internal admin page must never be requested: there, a plain GET can change things. |
| The request goes to exactly the address that was checked | So a name can't answer "public" for the check and "private" for the request. |
| `localhost`, one-word names, and names ending in `.local`, `.internal`, `.lan`, `.home`, `.corp`, `.test`, `.example`, `.invalid`, `.onion` are not even looked up | They don't lead to public websites. |
| Standard ports only (80, 443); no address written as a bare number (`2130706433`, `0177.0.0.1`, `127.1`) | Odd ports are admin panels and dev servers; odd number forms are a way round the address check. |
| An address with a key or token in it (`?token=`, `?code=`, `?key=`, `?reset=` and the like), or one that is an action (`/logout`, `/unsubscribe`), is not requested | Asking could use up a one-time link or do the thing. |
| HEAD first; a GET for one byte only when HEAD is refused (405 or 501); the body is never read | The lightest question that gets an answer. |
| At most four redirects, each one checked the same way; a redirect to anything not allowed is not followed | A public page can point at a private one. |
| 6 seconds to connect, 8 to answer; four at a time; one at a time per site with a second between; 500 pages unless `--max` says otherwise (3,000 at most); 15 minutes in all | Polite, and bounded. |
| User-Agent `bookmark-cleanup-link-check/1.0 (a person checking their own saved bookmarks; one request per page)` | Honest about what it is. |

What the answers mean:

- **ok**: any 2xx or 3xx at the end.
- **dead**: 404 or 410, or the domain doesn't exist (checked twice, and never believed when no name at all could be looked up, which means the computer is offline).
- **unreachable**: everything else. A timeout, a refused connection, a certificate problem, 401, 403, 429, 5xx, too many redirects, a redirect somewhere that isn't requested. These go to "Check these by hand" and are never offered for removal.
- **skipped**: not asked, with the reason.

Without `--person-agreed` it prints how many pages and sites it would ask, requests nothing and exits with code 2.

## links.json format

```json
{
  "schema": "bookmark-cleanup/links@1",
  "checked_on": "2026-10-13", "requested": 212, "sites": 148, "capped": false,
  "results": {"2": {"status": "ok", "why": "answers", "code": 200},
              "17": {"status": "dead", "why": "the site says the page is gone (404)", "code": 404},
              "23": {"status": "unreachable", "why": "no answer in time"},
              "31": {"status": "skipped", "why": "a private or local address"}}
}
```

Keys of `results` are bookmark ids from `bookmarks.json`. It holds no addresses and no page content.

## Importing the new file

The user does this. The new file is `bookmarks-tidy.html`; it is for importing, not for opening as a page. Menu names move a little between versions: look for Import, then the choice that says HTML file.

- **Chrome**: three dots > Bookmarks and lists > Import bookmarks and settings > Bookmarks HTML file > Choose file.
- **Edge**: three dots > Favorites > three dots > Import favorites > Import from other browsers > Favorites or bookmarks HTML file > Choose file.
- **Brave**: menu > Bookmarks > Import bookmarks and settings > Bookmarks HTML file.
- **Vivaldi**: File > Import from Applications or Files, then choose the bookmarks HTML file.
- **Firefox**: Bookmarks > Manage bookmarks > Import and Backup > Import Bookmarks from HTML.
- **Safari**: File > Import From > Bookmarks HTML File.

Importing adds; it doesn't replace. If the browser already has bookmarks, the imported ones arrive in a new folder (Chrome and Edge call it "Imported", or put it under Other bookmarks). In a browser with no bookmarks, Chrome puts them straight on the bookmarks bar.

## Replacing the old bookmarks instead of adding to them

Say this to the user in order. Each step is theirs.

1. Save a backup first with the browser's own Export bookmarks. Keep it.
2. Import `bookmarks-tidy.html`.
3. Look through the imported folder. Compare with `bookmarks-removed.html` if something seems missing: everything left out is in there, and it can be imported too.
4. When happy: in the bookmark manager, delete the old bookmarks and folders (everything except the imported folder), then drag the imported bookmarks-bar items onto the bar and the rest where they belong.
5. If sync is on, the deletions and the new bookmarks sync to their other devices. That is expected; the backup from step 1 is the way back.

Never offer to do steps 2 to 4 by editing the browser's files. That is the one thing this skill doesn't do.
