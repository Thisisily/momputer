"""Version the copied Pages demo's HTML asset URLs without changing native app files.

    python scripts/version-demo-assets.py site/demo

Only local script src and stylesheet href attributes in index.html change. Asset
filenames and contents stay intact, so CSS URLs and relative module imports resolve
from the same directories as before.
"""
import hashlib
import html
from html.parser import HTMLParser
from pathlib import Path
import re
import sys
from urllib.parse import parse_qsl, unquote, urlencode, urlsplit, urlunsplit


class AssetVersions(HTMLParser):
    def __init__(self, source, root):
        super().__init__(convert_charrefs=True)
        self.root = root.resolve()
        self.source = source
        self.edits = []
        self.line_offsets = [0]
        self.line_offsets.extend(match.end() for match in re.finditer("\n", source))

    def handle_starttag(self, tag, attrs):
        values = dict(attrs)
        if tag == "script":
            attribute = "src"
        elif tag == "link" and "stylesheet" in (values.get("rel") or "").lower().split():
            attribute = "href"
        else:
            return
        url = values.get(attribute)
        if not url:
            return
        parts = urlsplit(url)
        if parts.scheme or parts.netloc or not parts.path or parts.path.startswith("/"):
            return
        asset = (self.root / unquote(parts.path)).resolve()
        if not asset.is_relative_to(self.root):
            raise ValueError(f"Demo asset escapes its output directory: {url}")
        digest = hashlib.sha256(asset.read_bytes()).hexdigest()[:16]
        query = [(key, value) for key, value in parse_qsl(parts.query, keep_blank_values=True) if key != "v"]
        query.append(("v", digest))
        versioned = urlunsplit((parts.scheme, parts.netloc, parts.path, urlencode(query), parts.fragment))
        raw = self.get_starttag_text()
        # Preserve all surrounding markup, including quote style and attribute order.
        for match in re.finditer(r"([\w:-]+)\s*=\s*([\"'])(.*?)\2", raw, re.DOTALL):
            if match.group(1).lower() == attribute:
                line, column = self.getpos()
                start = self.line_offsets[line - 1] + column + match.start(3)
                end = self.line_offsets[line - 1] + column + match.end(3)
                self.edits.append((start, end, html.escape(versioned, quote=True)))
                return
        raise ValueError(f"Expected a quoted {attribute} attribute: {raw}")

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)

    def rendered(self):
        result = self.source
        for start, end, value in reversed(self.edits):
            result = result[:start] + value + result[end:]
        return result


def main():
    if len(sys.argv) != 2:
        sys.exit("Usage: python scripts/version-demo-assets.py <copied-demo-directory>")
    root = Path(sys.argv[1])
    index = root / "index.html"
    source = index.read_text(encoding="utf-8")
    parser = AssetVersions(source, root)
    parser.feed(source)
    parser.close()
    if not parser.edits:
        sys.exit("No local demo scripts or stylesheets found; nothing was published.")
    index.write_text(parser.rendered(), encoding="utf-8")
    print(f"Versioned {len(parser.edits)} local demo asset URLs in {index}")


if __name__ == "__main__":
    main()
