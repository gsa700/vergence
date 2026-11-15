#!/usr/bin/env python3
"""Builds data/words/en_US.txt, the spelling list Vergence ships (editor
milestone 1, step 7; Dave, 2026-11-15: "ship our own"), from a SCOWL
release (Spell Checker Oriented Word Lists, Kevin Atkinson's;
http://wordlist.aspell.net/).

    python3 tools/make-words.py /path/to/scowl-2020.12.07 > data/words/en_US.txt

What goes in, as Hunspell's en_US is made: the English and American lists
(words, capitalized words, contractions, abbreviations, proper names) and
the special hacker and Roman numeral lists, sizes 10 to 60. SCOWL writes
every form out (plurals, tenses, possessives), so no affix rules are needed.
The files are Latin-1; the list is UTF-8.

The list's form: comment lines start with '#'; a line "= N" starts the
words of size level N (10 the commonest ... 60), one word a line after it,
sorted. The level ranks suggestions and completions.
"""

import os
import re
import sys

SIZES = [10, 20, 35, 40, 50, 55, 60]
KINDS = ["words", "upper", "contractions", "abbreviations", "proper-names"]


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: make-words.py SCOWL-DIR")
    final = os.path.join(sys.argv[1], "final")
    level = {}
    for size in SIZES:
        names = [f"{spelling}-{kind}.{size}" for spelling in ("english", "american") for kind in KINDS]
        names += [f"special-hacker.{size}", f"special-roman-numerals.{size}"]
        for name in names:
            path = os.path.join(final, name)
            if not os.path.exists(path):
                continue
            with open(path, encoding="latin-1") as f:
                for line in f:
                    w = line.strip()
                    # A word of one line, no spaces; the lowest level it is in.
                    if not w or " " in w or len(w) > 64:
                        continue
                    if w not in level:
                        level[w] = size
    version = ""
    vpath = os.path.join(sys.argv[1], "VERSION")
    if os.path.exists(vpath):
        version = open(vpath, encoding="latin-1").read().strip()
    out = sys.stdout
    out.write("# Vergence's American English spelling list, made by tools/make-words.py\n")
    out.write(f"# from SCOWL {version} (sizes 10-60, as Hunspell's en_US). SCOWL is\n")
    out.write("# Copyright 2000-2018 Kevin Atkinson and others; its terms are in\n")
    out.write("# data/words/SCOWL-COPYRIGHT.txt, which goes with this file.\n")
    out.write(f"# {len(level)} words.\n")
    for size in SIZES:
        words = sorted(w for w, s in level.items() if s == size)
        if words:
            out.write(f"= {size}\n")
            out.write("".join(w + "\n" for w in words))


if __name__ == "__main__":
    main()
