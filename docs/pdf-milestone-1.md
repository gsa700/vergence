# PDF milestone 1: our own PDF engine, and PDFs in Close-Up

Status: **accepted 2026-11-19** (Dave: "close-up, drawn signature first,
placeholder for jpeg 2000"; the corpus: "use the PDFs in my Documents
folder for testing"). Steps 1 to 3 built 2026-11-19, step 4 part 1 the
same night, parts 2 and 3 too: step 4 done; step 5 under way (below).

## What Dave asked for

- Wishlist: "Add PDF Viewer (Editor?) to Close-Up or make standalone?"
- 2026-11-19: "obviously I could just use firefox to read pdf but that is a
  lot just for a document viewer?"; "I would prefer our own since it will
  be in our app and part of the vergence core app set."
- What "editor" means (agreed): highlight and notes, filling in forms,
  signing, reordering, rotating, deleting and merging pages. **Not**
  rewriting the text inside a PDF, and **not** making PDFs from scratch
  (LibreOffice's export and printing to PDF cover that).
- "lets plan to add a print section to settings and print to pdf in our
  system dialog then."

Answers apps-roadmap.md's question 2 (write our own) and its "PDF in
Close-Up" item.

## The plan in three milestones

1. **Viewing** (this document's steps): our own PDF reader and page
   renderer in a sandboxed helper; Close-Up shows PDFs with pages,
   thumbnails, zoom, search, copying text, links and the outline.
2. **Editing**: notes and highlights, forms, signing, pages; our own PDF
   writer, saving by incremental update (the original bytes kept, changes
   appended, as Acrobat and Preview do).
3. **Printing**: Vergence's own print dialog for every app, with **Save as
   PDF** through milestone 2's writer (a scene becomes a real PDF: shapes,
   embedded font subsets, pictures, selectable text; no CUPS needed);
   printers through CUPS where it runs (its local socket, IPP; driverless
   printers take PDF); a **Printing** page in Settings (printers, the
   default one, paper size, Save as PDF's folder). Checked 2026-11-19: no
   CUPS on Hubble; kepler and techbench have its tools, no scheduler
   running; `cups-pdf` nowhere. GTK and Qt apps keep their own dialogs.
   The test printer (Dave, 2026-11-19): the Brother HL-L6200DW laser on
   the LAN (driverless IPP, duplex), connected to all three test machines
   when milestone 3 comes.

## Why this shape

- **A PDF is untrusted input**, and a classic way in. Everything that reads
  a PDF runs in a helper with vergence-decode's limits (Capsicum on
  FreeBSD; no-new-privileges and seccomp on Linux; no file system, no
  network, CPU, memory and descriptor limits). Unlike vergence-decode's
  one picture and done, a document stays open: the helper lives while its
  window does and answers requests on its pipe (a page at a size, a page's
  text, a search). A crash or hang is an error in the window, never a
  crash of Close-Up.
- **Pixels out, not scenes.** The helper draws pages to pixels at the size
  asked, in tiles when zoomed in. PDF needs what our scenes do not have
  (arbitrary curved paths, even-odd fills, clipping paths, gradients,
  blend modes, soft masks), and pixels keep the pipe simple and the
  helper's output easy to check. Text comes back separately as positioned
  characters, for selecting, copying and search.
- **Real files first.** The order of the steps is the order the PDFs Dave
  actually opens need things: manuals, data sheets, invoices, forms,
  scanned pages, ham radio documents (ARRL, manufacturer manuals).

## Steps (milestone 1)

1. **crates/vergence-pdf: reading the file.** The lexer and objects
   (numbers, strings, names, arrays, dictionaries, streams, references);
   the cross-reference table and streams (PDF 1.5), object streams;
   incremental updates (the newest object wins); **repair** when the table
   is wrong (scan for `N 0 obj`), which real files need often; the page
   tree with inheritance; stream filters: Flate (our own inflate, with
   PNG predictors), LZW, ASCIIHex, ASCII85, RunLength. Every length,
   offset, depth and count bounded; fuzzed like sfnt.rs. Encrypted files:
   the standard security handler (RC4 and AES-128/256, MD5/SHA-2 our own),
   so the many files encrypted with an empty password open; a password
   asked for when one is needed.
2. **crates/vergence-raster: our own path renderer.** Lines and Bézier
   curves flattened, non-zero and even-odd fills with exact-area
   anti-aliasing (as the CPU renderer's edges), strokes (widths, caps,
   joins, miter limits, dashes) turned into fills, clipping by any path,
   images drawn through a matrix with smoothing. Pure, tested against
   reference pictures. (Later it can stand in for resvg's renderer:
   borrowed-review.md.)
3. **Pages drawn.** The content stream interpreter: graphics state,
   transforms, paths, colors (gray, RGB, CMYK by the simple formula,
   indexed, ICC-based streams taken by their component count), shading
   (axial, radial, function-based), Form XObjects, transparency groups and
   soft masks, the common blend modes. Annotations' appearance streams
   drawn on top (stamps, filled forms as they were saved).
4. **Text drawn.** Fonts: embedded TrueType and OpenType (outlines through
   what vergence-text uses), bare CFF (FontFile3, wrapped), Type 1
   (FontFile; our own reader for its charstrings, eexec decryption),
   Type 3 (glyphs as content streams); simple and composite (Type 0, CID)
   fonts, encodings and differences, `ToUnicode` maps for the text layer;
   the 14 standard fonts drawn with substitutes that keep their widths.
   Rendering modes (fill, stroke, clip, invisible: scanned pages' OCR
   text). Glyphs through text milestone 1's coverage curve, so PDF text
   looks like the rest of Vergence.
5. **Pictures in PDFs.** JPEG (DCT) through vergence-decode's decoder,
   inside the same helper; Flate pictures; masks and soft masks; 1-bit
   pictures. CCITT fax (G3/G4) and JBIG2 for scanned documents, our own.
   JPEG 2000: a placeholder (a gray box saying what it is) until later
   (Dave, 2026-11-19).
6. **Close-Up's PDF view.** Pages in a column, scrolled continuously;
   zoom (fit width, fit page, actual size, the keys and pinch as for
   pictures), drawn sharp at every zoom and scale (pages redrawn at the
   screen's pixels, tiles while zoomed in, the page in view first);
   thumbnails in a sidebar; the outline (bookmarks) in the sidebar too;
   links (inside the document; outside ones open through vergence-open,
   asked first); go to page; Left/Right a page at a time when the whole
   page fits. Remembers each document's page and zoom.
7. **Text: select, copy, search.** Drag to select across lines in reading
   order (by the text layer's positions), copy to the clipboard, Find with
   every match marked and stepped through; a document with no text layer
   says so (a scan) instead of finding nothing silently.
8. **File Viewer and Preview.** PDF thumbnails (the first page) in File
   Viewer and the desktop, through the same helper; Space's Preview shows
   the first pages; "Open in Close-Up". vergence-mimeapps: Close-Up the
   default for application/pdf.
9. **Checks.** A corpus of PDFs kept outside the repository (Dave's
   everyday files, PDF 1.3 to 2.0, scans, forms, encrypted, broken), each
   page drawn by us and compared by eye against Firefox's pdf.js for the
   first pass, then kept as reference pictures for regressions; the fuzz
   tests; a timing check (a typical page drawn in well under 100 ms on
   kepler). On Hubble, kepler and techbench.

## Not in milestone 1

Editing, the writer and printing (milestones 2 and 3); digital signature
checking (shown as "signed" with no claim until milestone 2); 3D, media,
JavaScript (never run), XFA forms (rare, Adobe-only); right-to-left text
selection order; color management beyond the simple formulas.

## Questions for Dave

1. *Answered: Close-Up.* **Close-Up or an app of its own?** Recommended: Close-Up, as the
   wishlist suggests and Preview does on the Mac (one viewer for pictures
   and documents); its window grows a sidebar for PDFs only.
2. *Answered: a placeholder for now.* **JPEG 2000** (scanned documents, some archives): our own decoder (a
   large job, wavelet and entropy coding) late in the milestone, or show
   such pictures as a gray box with a note until then?
3. *Answered: drawn signature first.* **Signing in milestone 2:** a drawn or typed signature placed on the
   page (what most people mean, and Preview's), or also cryptographic
   signatures (X.509 certificates, CMS: a large crypto job, and a
   borrowing question if not our own)? Recommended: drawn first.
4. *Answered: the PDFs in Dave's Documents folder on Control, except
   two folders he excluded (1,545 files, 3.7 GB on 2026-11-19), copied to the dev boxes for checks; the
   files, their pictures and anything drawn from them stay out of the
   repository and off GitHub.* **The corpus:** which PDFs of yours can be used for checks (they stay
   on Control and the dev boxes, never in the repository)?

## Progress

- **Step 1, part 1** (2026-11-19 night): crates/vergence-pdf, no
  dependencies: lexer.rs (objects, strings with every escape, names with
  #xx, nesting capped at 64, stray tokens skipped as readers do), xref.rs
  (tables, xref streams, hybrid /XRefStm, /Prev chains with loop and depth
  limits, offsets counted from a late %PDF header, the repair scan with
  the trailer or catalog found), lib.rs (Document: objects at offsets and
  in object streams, a wrong offset looked up by scanning, reference
  cycles cut, the page tree with inheritance and cycle cuts, content
  streams joined), filter/ (our own inflate, LZW with EarlyChange,
  ASCIIHex, ASCII85, RunLength, PNG and TIFF predictors; image filters left
  named for step 5). 23 tests, the fuzz test included, on Hubble, kepler
  and techbench; examples/pdfcorpus.rs reads a folder of PDFs.
  **The corpus** (1,544 of Dave's PDFs on Hubble, 25 s): all opened, no
  panics, 232,890 pages; the 1,443 unencrypted files have pages and every
  content stream decodes cleanly. Everything left (64 files with no pages
  found, 2,805 content streams) is in the 101 encrypted files: part 2.
  Slowest: a 2.1 s technical book (to look at with step 3).
- **Step 1, part 2** (2026-11-19 night): crypt/, the standard security
  handler, revisions 2 to 6: our own MD5, SHA-256/384/512 (constants
  computed exactly, not typed), RC4 and AES-128/256 (S-box from its
  definition; CBC both ways for revision 6's hash), checked against the
  published vectors; keys by algorithms 2, 2.A and 2.B, the user password
  then the owner's, crypt filters (V2, AESV2, AESV3, Identity), unencrypted
  xref streams and metadata kept so. The empty user password is tried at
  open; Document::needs_password and unlock for the rest. 29 tests on all
  three. **The corpus**: 100 of the 101 encrypted files open by
  themselves (revision 2: 16, 3: 22, 4: 62, 6: 1); one personal document
  has a real user password and correctly asks for it. 238,301 pages,
  every content stream decoding cleanly.
- **Step 2** (2026-11-19 night): crates/vergence-raster, no dependencies:
  geom.rs (affine transforms composed as PDF's `cm`, paths of lines and
  cubics, curves flattened by Wang's bound to 0.1 device pixel),
  raster.rs (exact-area coverage accumulation, font-rs's method;
  non-zero and even-odd), stroke.rs (quads, miter/round/bevel joins with
  the miter limit, butt/round/square caps, dashes with phase, worked out
  in user space so a stretched matrix gives an elliptical pen; never
  thinner than one device pixel; a dot for a zero-length round cap),
  lib.rs (Canvas: fills, strokes, a per-pixel paint for shadings, Mask
  clips intersected path by path, images through any transform with
  smoothing). Bounded and fuzzed; 20 tests on all three;
  examples/rastercard.rs draws a test card (checked by eye).
- **Step 3** (2026-11-19 night): vergence-pdf's function.rs (types 0, 2, 3
  and the PostScript calculator, bounded), color.rs (every space to sRGB;
  ICC by component count, CMYK by formula), content.rs (operations and
  inline images), image.rs (1 to 16 bits, Decode, stencils, color-key,
  explicit and soft masks; picture codecs left for step 5), render.rs (the
  interpreter: paths, painting, clipping after paint, the q/Q stack,
  ExtGState, forms nested with cycles cut, images, axial, radial and
  function-based shadings by `sh` and as patterns, tiling patterns drawn
  once and repeated, constant alpha, annotations' normal appearances, page
  rotation). vergence-raster gained Clip (a rectangle kept exact, else a
  mask). 45 tests in vergence-pdf on all three. **The corpus**: every first
  page of the 1,544 files drawn, no panics, about 40 ms a page; checked by
  eye on Victron, ICOM and Motorola manuals (vector art, logos, shaded
  illustrations right; text missing until step 4, JPEGs gray boxes until
  step 5). Not yet: blend modes, soft-mask groups, knockout, mesh
  shadings (4-7). Slowest pages about 3 s (many non-rectangular clips, each
  a full-page mask: to bound by box in the speed-up).
- **Step 4, part 1** (2026-11-19 night): font/ in vergence-pdf:
  truetype.rs (our own: head, maxp, loca, glyf with composites, cmap
  formats 0, 4, 6, 12, hmtx), encoding.rs (Standard, WinAnsi and MacRoman
  and glyph names to Unicode, generated by tools/make-pdf-encodings.py
  from Python's codecs), mod.rs (simple fonts with /Differences and
  /Widths; Type 0 with Identity or embedded CMaps, /W, CIDToGIDMap;
  glyph choice per §9.6.5.4; substitutes for fonts not embedded through
  vergence-text's font database, metric-compatible first, stretched to
  /Widths). render.rs: the text state in the graphics state, every
  positioning and showing operator, modes 0 to 7 (text clips at ET).
  Seen by eye: the ICOM service manual's cover lettering, a Victron table
  of contents (leaders and page numbers aligned). Every first page of the
  corpus still draws, no panics. Left: CFF (FontFile3 and in OpenType),
  Type 1, Type 3, ToUnicode, vertical writing, predefined CMaps other than
  Identity, post-table glyph names. Substitutes: Liberation is installed
  only on Hubble; elsewhere they fall back to Inter and JetBrains Mono
  (question for Dave: ship Liberation, OFL, like Inter?). In the
  sandboxed helper (step 6) substitutes must be opened before it limits
  itself.
- **Step 4, part 2** (2026-11-19 night): font/cff.rs, our own CFF reader
  and Type 2 charstring interpreter (INDEXes, DICTs with reals, charsets
  0/1/2, encodings with supplements, private dicts and subroutines with
  their bias, CID-keyed fonts with FDArray and FDSelect, hint masks, widths,
  flex, `endchar` accents); FontFile3 /Type1C and /CIDFontType0C and
  OpenType fonts with a CFF table. A hand-made CFF in the tests, and a
  fuzz test. Seen by eye: the ICOM cover's model name in its logo font, a
  Motorola R56 page of tables and body text. Corpus: every first page,
  no panics. CFF standard strings 229-390 (the expert set) not kept.
  Liberation 2.1.5 shipped in data/fonts the same night (Dave: "ship
  Liberation").
- **Step 4, part 3** (2026-11-19 night): font/type1.rs, our own Type 1
  reader (the clear part's encoding and matrix, eexec binary or hex,
  lenIV, Subrs, CharStrings) and charstring interpreter (flex and hint
  replacement through othersubrs 0-3, div, seac); Type 3 glyphs run as
  content streams in glyph space; ToUnicode (bfchar, bfrange with arrays
  and counting) and Font::text for the text layer; Symbol and ZapfDingbats
  not embedded read through their own encodings (Greek, math signs,
  bullets). Seen by eye: the Kantronics KPC-3+ sheet (Type 1), an Omron
  relay datasheet (Type 3 and Symbol/Dingbats: the bullets and "VSWR
  ≤1.2" right). Corpus: no panics. 54 tests on all three.
- **Step 5, part 1** (2026-11-20 night): JPEG. vergence-decode's
  jpeg_samples (the image crate, as the plan says; no EXIF turning, 256 MB
  cap) gives a JPEG's samples; image.rs runs them through the usual path,
  the PDF's color space kept when its component count matches (CMYK
  JPEGs come converted to RGB), masks kept. Seen: the Motorola R56
  cover's photo collage, the Omron relay photo. In the corpus: DCTDecode
  in 679 files, CCITTFaxDecode in 160, JPXDecode in 30 (placeholder, as
  Dave chose), JBIG2Decode in 12. CCITT next.
- **Step 5, part 2** (2026-11-20 night): ccitt.rs, our own CCITT fax
  decoder (T.4/T.6: Group 3 1D and 2D, Group 4; EndOfLine,
  EncodedByteAlign, EndOfBlock, BlackIs1), the run-length tables written
  as bit strings. Tested against libtiff: one 3000 x 40 bitmap with runs
  past the extended codes, encoded by tiffcp in all three schemes
  (testdata/ccitt-*), decodes to the bitmap pixel for pixel; a prefix-code
  check and a fuzz test. Seen: the ARRL archive's QST TS-850S review
  cover (scanned ad column). Corpus: no panics. Left in step 5: JBIG2 (12
  files), JPEG 2000 placeholder (30 files, as Dave chose).
- **Step 5, part 3** (2026-11-20 night): jbig2.rs, our own JBIG2 decoder:
  the MQ arithmetic decoder (T.88 Annex H.2's test sequence decodes
  exactly), integer and symbol-ID decoding, generic regions (templates
  0-3, TPGDON, MMR through ccitt.rs), refinement, arithmetic symbol
  dictionaries with aggregation, text regions, stripes, /JBIG2Globals;
  bounded (pixels, symbols, instances, a decision budget) and fuzzed. In
  the corpus all 12 JBIG2 files are arithmetic coded; one (684938.pdf)
  also has Huffman text regions, not yet decoded, and no file uses
  halftones. Seen: the Transistor Radio Handbook (symbols and text), the
  TS-830S service manual (MMR and arithmetic), the Collins 30L-1 book
  cover. Corpus: no panics; 63 tests on all three. Left in step 5: the
  JPEG 2000 placeholder (30 files; the Icom receivers brochure is nearly
  all JPX), Huffman JBIG2.
- **Step 6, part 1** (2026-11-20 night): the sandboxed helper.
  vergence-pdf's helper.rs: `libexec/vergence/vergence-pdf` reads the
  substitute fonts (font::preload_substitutes), then confines itself with
  vergence-decode's `confine` (now public, with the CPU time as a
  parameter: 4 hours over a helper's life, 10 s for pictures as before),
  then serves requests on its pipe: the document's bytes (it opens no
  file), a password, a page or a tile at a scale (render_region, new).
  The client `Viewer` times each answer (30 s), and after a crash or hang
  starts a new helper with the document and password again; a panic while
  drawing refuses that request only. tests/helper.rs runs the real
  confined helper (seccomp on Hubble, Capsicum on kepler and techbench);
  examples/pdfhelper draws through it (a non-embedded Helvetica drawn by
  its substitute under Capsicum on kepler). install.sh installs it.
- **Step 6, part 2** (2026-11-20 night): Close-Up opens PDFs
  (vergence-closeup pdf.rs): pages in a column, scrolled continuously; fit
  to the width (the default), the whole page (W), actual size (1), zoom
  (+/-, Ctrl+wheel about the pointer); the wheel, arrows, Page Up/Down,
  Space and dragging scroll; Home/End; Left/Right a page when it fits
  whole; Previous/Next page in the toolbar and the app menu; "Page N of
  M" and the zoom in the status line. A worker thread owns the helper and
  draws the pages in sight at the screen's pixels, the middle one first
  and the next one ahead; a page shows its last picture scaled until the
  sharp one comes; far pages are forgotten; a page the helper could not
  draw is tinted and says why in the status line. Seen in a headless
  session on Hubble: the Motorola R56 manual (518 pages) at width and
  page fits, stepping pages. Still to come in step 6: tiles past 16 M
  pixels a page, the password field, the sidebar (thumbnails, outline),
  links, go to page, each document's page and zoom remembered.
- **Step 6, part 3** (2026-11-20 night): deep zoom in tiles (past 16 M
  pixels a page, the part in sight is drawn in 512-pixel tiles at the
  screen's pixels over the page picture; the R56 manual crisp at 723% with
  no seams); a locked PDF shows a password field (the helper unlocks it,
  "That password does not open it." otherwise; tried with a generated RC4
  R3 file); each document's page and zoom remembered in
  ~/.local/state/vergence/closeup-pdf-places (owner-only, 500 at most,
  saved 0.7 s after they stop changing; reopening goes to the page's top,
  not the exact spot in it); the status line's page number is a field
  (type a number, Enter). Still in step 6: the sidebar (thumbnails,
  outline), links.
- **Step 6, part 4** (2026-11-20 night): the outline and links.
  vergence-pdf's outline.rs: text strings (UTF-16BE, UTF-8, PDFDocEncoding),
  destinations (arrays with XYZ/FitH/FitR tops turned into points from
  the page's top as shown, names through /Dests and the /Names tree with
  its limits, dictionaries with /D), the outline tree (cycles cut, depth
  and count bounded), each page's links (GoTo, URI, the Named page
  actions); the helper's `O` and `L` requests. Corpus: 306 files with an
  outline, 48,715 entries, 195,398 links read, no panics. Close-Up: a
  sidebar (S, the toolbar's first button, the app menu) with the pages'
  thumbnails (drawn on the worker after the view's pages, the current one
  marked and kept in sight; click to go) and, where there is one, the
  outline (Contents: indented, cut with "…", the entry for the place in
  view marked and scrolled to); a click on a link goes to its place (the
  spot on the page when it names one); an address outside the document is
  asked about in the status line first, and only http(s), mailto and ftp
  open (through vergence-open). Seen in a headless session: the R56
  manual's table of contents links, its outline, thumbnails. Step 6 is
  done but for checking it on the real screens.
- **Step 7** (2026-11-20 night): text. vergence-pdf's text.rs and
  render.rs's page_text: the page run without painting (no images
  decoded), every character's text (ToUnicode or encoding) and box,
  overprinted characters kept once; joined with the spaces and line
  breaks the positions imply; finding with case and white space folded;
  a selection's text and boxes; the helper's `T` request; examples/pdftext.
  4 ms for an R56 page. Close-Up: a drag that starts on text selects it
  (across pages, in drawn order), anywhere else pans; a click clears it
  or follows a link; Ctrl+C copies, Ctrl+A selects the page in view,
  Escape clears; Find (Ctrl+F, the toolbar's search button, starting from
  a short selection) searches every page on the worker after the view,
  marks every match (the shown one orange), Enter/Ctrl+G/F3 next,
  Shift for previous, "N of M found (searching, page X of Y)", and a
  document without text says it is pictures (a scan) instead of "Not
  found". Seen headless on Hubble: "grounding" 2,040 times in the R56
  manual in about 5 s; a selection copied and pasted into Edit, lines
  intact. Tests on all four machines (apollo, Fedora, joined).
- **Step 8** (2026-11-20 night): PDFs in FileView and on the desktop.
  vergence-thumbs: has_thumbnail (pictures and PDFs) and pdf_page (the
  first page drawn by the confined helper to fit a size); PDF thumbnails
  go into the shared freedesktop cache like pictures', a PDF that needs a
  password or will not draw is marked failed and keeps its icon. FileView:
  the icon view's thumbnails, the side pane and Space's preview (the
  first page at up to 1200 px) and "Open in Close-Up" for PDFs; the
  desktop's icons the same. Close-Up's desktop entry lists
  application/pdf ("Picture and PDF Viewer"), so it is the default for
  PDFs (our own apps first; a choice made in Vergence still wins). Seen
  headless on Hubble: a folder of the R56 manual, the Omron datasheet, a
  QST scan (CCITT), the Transistor Radio Handbook (JBIG2) and the locked
  test file (the PDF icon): thumbnails, and the R56's first page in
  Space's preview. Space shows the first page only for now. Tests on all
  four machines.
