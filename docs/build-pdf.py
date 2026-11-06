"""Render the whitepaper Markdown to print-ready HTML (then Edge prints it to PDF)."""
import re
import sys
import markdown

src, out = sys.argv[1], sys.argv[2]
text = open(src, encoding="utf-8").read()
text = re.sub(r"\A---\n.*?\n---\n", "", text, flags=re.S)  # drop YAML front matter

# Split the title block (everything before the Abstract) from the body.
head, body = text.split("## Abstract", 1)
head_lines = [l for l in head.strip().splitlines() if l.strip() and l.strip() != "---"]
title = head_lines[0].lstrip("# ").strip()
subtitle = head_lines[1].strip("* ").strip()
byline = head_lines[2].strip()

md = markdown.Markdown(extensions=["tables", "toc", "sane_lists", "smarty"],
                       extension_configs={"toc": {"toc_depth": "2-2"}})
html_body = md.convert("## Abstract" + body)
toc = md.toc

css = """
@page { size: Letter; margin: 22mm 20mm 20mm 20mm; }
:root { --ink:#1b1f24; --muted:#5b6470; --accent:#1f4e79; --rule:#d5dbe2; --band:#f3f6f9; }
html { font-size: 10.6pt; -webkit-print-color-adjust: exact; print-color-adjust: exact; }
body { font-family: "Cambria", "Georgia", serif; font-variant-numeric: lining-nums; color: var(--ink); line-height: 1.5; margin: 0; }
h1, h2, h3, .sub, .byline, th, .toc, .label { font-family: "Segoe UI", "Helvetica Neue", Arial, sans-serif; }
.cover { height: 218mm; display: flex; flex-direction: column; justify-content: center; page-break-after: always;
         border-left: 6px solid var(--accent); padding-left: 14mm; }
.cover .label { text-transform: uppercase; letter-spacing: .18em; color: var(--accent); font-size: 9pt; font-weight: 600; }
.cover h1 { font-size: 40pt; margin: 6mm 0 4mm; color: var(--ink); letter-spacing: -.01em; }
.cover .sub { font-size: 15pt; color: var(--muted); max-width: 140mm; line-height: 1.35; }
.cover .byline { margin-top: 18mm; font-size: 10.5pt; color: var(--muted); }
.cover .toc { margin-top: 14mm; font-size: 10pt; }
.cover .toc ul { list-style: none; padding: 0; margin: 0; }
.cover .toc li { margin: 0 0 1.6mm; break-inside: avoid; }
.cover .toc a { color: var(--ink); text-decoration: none; }
h2 { font-size: 16pt; color: var(--accent); margin: 9mm 0 3mm; padding-bottom: 1.5mm; border-bottom: 1.5px solid var(--rule);
     page-break-after: avoid; break-after: avoid; }
h3 { font-size: 12pt; margin: 6mm 0 2mm; page-break-after: avoid; break-after: avoid; }
h2#abstract + p, h2#abstract ~ p:first-of-type { }
p { margin: 0 0 2.6mm; orphans: 3; widows: 3; }
ul, ol { margin: 0 0 3mm; padding-left: 6mm; }
li { margin-bottom: 1.2mm; }
strong { color: #0f141a; }
code { font-family: "Cascadia Mono", Consolas, monospace; font-size: 8.8pt; background: var(--band); padding: 0 .25em; border-radius: 2px; }
table { width: 100%; border-collapse: collapse; margin: 3mm 0 4mm; font-size: 8.9pt; line-height: 1.35;
        page-break-inside: avoid; break-inside: avoid; }
th { background: var(--accent); color: #fff; text-align: left; font-weight: 600; padding: 1.6mm 2mm; }
td { padding: 1.5mm 2mm; border-bottom: 1px solid var(--rule); vertical-align: top; }
tr:nth-child(even) td { background: var(--band); }
hr { display: none; }
em { color: #2b333c; }
"""

html = f"""<!doctype html><html lang="en"><head><meta charset="utf-8">
<title>{title}</title><style>{css}</style></head><body>
<section class="cover">
  <div class="label">Whitepaper · Draft</div>
  <h1>{title}</h1>
  <div class="sub">{subtitle}</div>
  <div class="byline">{byline}</div>
  <div class="toc">{toc}</div>
</section>
{html_body}
</body></html>"""
open(out, "w", encoding="utf-8").write(html)
print("wrote", out)
