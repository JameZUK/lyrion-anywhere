#!/usr/bin/env python3
"""Check that the documentation's relative links, anchors and images resolve.

Scans README.md, SPEC.md, CHANGELOG.md and every Markdown file under docs/ (the
gitignored docs/reviews/ excepted) for Markdown links and images and for HTML
<a href> and <img src>. Each relative target must exist in the repository, each
`#anchor` must match a heading in the target file (GitHub's slug rules), and each
SVG image must parse as XML. External URLs are not fetched.

    python3 tools/check-docs.py              # check the repository
    python3 tools/check-docs.py --self-test  # prove the checker catches breakage

Exits 1 with one line per problem when anything is broken.
"""

from __future__ import annotations

import os
import re
import sys
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

MD_LINK = re.compile(r"!?\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
HTML_REF = re.compile(r"<(?:a|img)\b[^>]*?\b(?:href|src)=\"([^\"]+)\"", re.IGNORECASE)
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")
EXTERNAL = re.compile(r"^(?:[a-z][a-z0-9+.-]*:|//)", re.IGNORECASE)
FENCE = re.compile(r"^\s*(```|~~~)")


def slugify(heading: str) -> str:
    """GitHub's heading-to-anchor rule, close enough for this repository."""
    text = re.sub(r"`([^`]*)`", r"\1", heading)
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = text.replace("*", "").replace("~", "")
    text = text.strip().lower()
    text = "".join(ch for ch in text if ch.isalnum() or ch in " -_")
    return text.replace(" ", "-")


def markdown_lines(path: Path):
    """Yield (line_number, line) outside fenced code blocks."""
    fence = None
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        match = FENCE.match(line)
        if match:
            marker = match.group(1)
            if fence is None:
                fence = marker
            elif marker == fence:
                fence = None
            continue
        if fence is None:
            yield number, line


def anchors_in(path: Path) -> set[str]:
    seen: dict[str, int] = {}
    anchors: set[str] = set()
    for _, line in markdown_lines(path):
        match = HEADING.match(line)
        if not match:
            continue
        slug = slugify(match.group(2))
        count = seen.get(slug, 0)
        seen[slug] = count + 1
        anchors.add(slug if count == 0 else f"{slug}-{count}")
    return anchors


def references_in(path: Path):
    for number, line in markdown_lines(path):
        for match in MD_LINK.finditer(line):
            yield number, match.group(1)
        for match in HTML_REF.finditer(line):
            yield number, match.group(1)


def documents(root: Path) -> list[Path]:
    files = [root / name for name in ("README.md", "SPEC.md", "CHANGELOG.md") if (root / name).is_file()]
    docs = root / "docs"
    if docs.is_dir():
        for path in sorted(docs.rglob("*.md")):
            if "reviews" in path.relative_to(docs).parts:
                continue
            files.append(path)
    return files


def check(root: Path) -> tuple[int, list[str]]:
    problems: list[str] = []
    checked = 0
    anchor_cache: dict[Path, set[str]] = {}
    for doc in documents(root):
        for number, target in references_in(doc):
            where = f"{doc.relative_to(root)}:{number}"
            if EXTERNAL.match(target):
                continue
            checked += 1
            target = target.replace("&amp;", "&")
            path_part, _, anchor = target.partition("#")
            if path_part:
                resolved = (doc.parent / path_part).resolve()
                try:
                    resolved.relative_to(root.resolve())
                except ValueError:
                    problems.append(f"{where}: {target} points outside the repository")
                    continue
                if not resolved.exists():
                    problems.append(f"{where}: {target} does not exist")
                    continue
                if resolved.suffix.lower() == ".svg":
                    try:
                        ET.parse(resolved)
                    except ET.ParseError as error:
                        problems.append(f"{where}: {target} is not well-formed XML ({error})")
                        continue
            else:
                resolved = doc
            if anchor:
                if resolved.suffix.lower() != ".md":
                    continue
                if resolved not in anchor_cache:
                    anchor_cache[resolved] = anchors_in(resolved)
                if anchor.lower() not in anchor_cache[resolved]:
                    problems.append(f"{where}: {target} has no heading for #{anchor}")
    return checked, problems


def self_test() -> int:
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        (root / "docs").mkdir()
        (root / "docs" / "images").mkdir()
        (root / "docs" / "images" / "good.svg").write_text('<svg xmlns="http://www.w3.org/2000/svg"/>')
        (root / "docs" / "images" / "bad.svg").write_text("<svg><unclosed></svg>")
        (root / "docs" / "GUIDE.md").write_text("# Guide\n\n## Set-up and *use*\n\n## Set-up and use\n")
        (root / "README.md").write_text(
            "# Front\n\n"
            "[ok](docs/GUIDE.md) [ok](docs/GUIDE.md#set-up-and-use) [ok](docs/GUIDE.md#set-up-and-use-1) [ok](#front)\n"
            '<img src="docs/images/good.svg"> [ext](https://example.com/x.md) [mail](mailto:a@example.com)\n'
            "```\n[not checked](nowhere.md)\n```\n"
            "[missing](docs/NOPE.md) [anchor](docs/GUIDE.md#nope) [self](#nope)\n"
            '<img src="docs/images/bad.svg"> [escape](../outside.md)\n'
        )
        checked, problems = check(root)
    expected = ["NOPE.md does not exist", "no heading for #nope", "no heading for #nope", "not well-formed XML", "outside the repository"]
    found = sorted(problems)
    ok = checked == 10 and len(found) == 5 and all(any(e in p for p in found) for e in expected)
    if not ok:
        print(f"self-test failed: checked={checked}", *found, sep="\n")
        return 1
    print("self-test passed")
    return 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    root = Path(argv[1]) if len(argv) > 1 else Path(__file__).resolve().parent.parent
    checked, problems = check(root)
    for problem in problems:
        print(problem)
    print(f"{checked} relative links, anchors and images checked, {len(problems)} broken")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
