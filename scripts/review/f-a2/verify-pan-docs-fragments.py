#!/usr/bin/env python3
"""Verify F-A2 Pan Docs citations resolve to concrete HTML fragments."""

from __future__ import annotations

from html.parser import HTMLParser
from urllib.request import Request, urlopen


USER_AGENT = "gbllm-f-a2-review-fragment-check"

CITATIONS = (
    (
        "cartridge header logo",
        "https://gbdev.io/pandocs/The_Cartridge_Header.html#0104-0133--nintendo-logo",
    ),
    (
        "cartridge type",
        "https://gbdev.io/pandocs/The_Cartridge_Header.html#0147--cartridge-type",
    ),
    (
        "rom size",
        "https://gbdev.io/pandocs/The_Cartridge_Header.html#0148--rom-size",
    ),
    (
        "ram size",
        "https://gbdev.io/pandocs/The_Cartridge_Header.html#0149--ram-size",
    ),
    (
        "destination code",
        "https://gbdev.io/pandocs/The_Cartridge_Header.html#014a--destination-code",
    ),
    ("memory map", "https://gbdev.io/pandocs/Memory_Map.html#memory-map"),
    ("echo ram", "https://gbdev.io/pandocs/Memory_Map.html#echo-ram"),
    ("unmapped range", "https://gbdev.io/pandocs/Memory_Map.html#fea0feff-range"),
    ("mbc5 registers", "https://gbdev.io/pandocs/MBC5.html#registers"),
    (
        "mbc5 ram enable",
        "https://gbdev.io/pandocs/MBC5.html#0000-1fff---ram-enable-write-only",
    ),
    ("ppu modes", "https://gbdev.io/pandocs/Rendering.html#ppu-modes"),
    (
        "interrupt enable",
        "https://gbdev.io/pandocs/Interrupts.html#ffff--ie-interrupt-enable",
    ),
    (
        "interrupt flags",
        "https://gbdev.io/pandocs/Interrupts.html#ff0f--if-interrupt-flag",
    ),
    (
        "interrupt priorities",
        "https://gbdev.io/pandocs/Interrupts.html#interrupt-priorities",
    ),
    (
        "divider",
        "https://gbdev.io/pandocs/Timer_and_Divider_Registers.html#ff04--div-divider-register",
    ),
    (
        "timer control",
        "https://gbdev.io/pandocs/Timer_and_Divider_Registers.html#ff07--tac-timer-control",
    ),
    ("joypad", "https://gbdev.io/pandocs/Joypad_Input.html#ff00--p1joyp-joypad"),
)


class IdCollector(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.ids: set[str] = set()

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        for key, value in attrs:
            if key in {"id", "name"} and value:
                self.ids.add(value)


def main() -> int:
    pages: dict[str, set[str]] = {}
    failures: list[str] = []

    for label, url in CITATIONS:
        page, fragment = url.split("#", 1)
        if page not in pages:
            request = Request(page, headers={"User-Agent": USER_AGENT})
            with urlopen(request, timeout=30) as response:
                html = response.read().decode("utf-8", errors="replace")
            collector = IdCollector()
            collector.feed(html)
            pages[page] = collector.ids
        if fragment not in pages[page]:
            failures.append(f"{label}: missing #{fragment} in {page}")

    if failures:
        print("Pan Docs fragment verification failed:")
        print("\n".join(failures))
        return 1

    print(f"Verified {len(CITATIONS)} Pan Docs citation fragments.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
