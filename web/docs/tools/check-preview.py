"""Check the approved preview inventory, question descriptions and local links."""

from __future__ import annotations

import argparse
import json
import re
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urljoin, urlsplit


class Links(HTMLParser):
    def __init__(self, text: str) -> None:
        super().__init__()
        self.ids: set[str] = set()
        self.links: list[str] = []
        self.feed(text)

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attributes = dict(attrs)
        if identifier := attributes.get('id'):
            self.ids.add(identifier)
        if tag == 'a' and (href := attributes.get('href')):
            self.links.append(href)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    site = Path(__file__).resolve().parents[1]
    inventory = json.loads((site / 'approved-docs.mjs').read_text().split('export const approvedDocs = ', 1)[1].rstrip().removesuffix(';'))
    errors = []
    pages: dict[str, Links] = {}
    for path in args.output.rglob('*.html'):
        route = '/' + str(path.relative_to(args.output)).removesuffix('index.html')
        pages[route] = Links(path.read_text())
    if len(inventory) != 36 or len(set(inventory)) != 36:
        errors.append('approved inventory must contain 36 unique source paths')
    for source in inventory:
        text = (site / 'src/content/docs' / source).read_text()
        description = re.search(r'^description: (.+)$', text, re.MULTILINE)
        if not description or not description[1].strip('"\'').endswith('?'):
            errors.append(f'{source}: description must be the reader question')
        route = '/' + re.sub(r'\.mdx?$', '', source).removesuffix('/index') + '/'
        if route not in pages:
            errors.append(f'{source}: missing built route {route}')
    for route, page in pages.items():
        for link in page.links:
            target = urlsplit(urljoin('https://docs.capsem.org' + route, link))
            if target.scheme not in ('http', 'https') or target.netloc != 'docs.capsem.org':
                continue
            if target.path not in pages:
                if not (args.output / unquote(target.path).lstrip('/')).exists():
                    errors.append(f'{route}: missing destination {link}')
            elif target.fragment and unquote(target.fragment) not in pages[target.path].ids:
                errors.append(f'{route}: missing fragment {link}')
    for error in sorted(set(errors)):
        print(error)
    if not errors:
        print(f'Checked {len(inventory)} approved pages and local links in {len(pages)} HTML files.')
    return int(bool(errors))


if __name__ == '__main__':
    raise SystemExit(main())
