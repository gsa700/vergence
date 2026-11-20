# PDF milestone 1: our own PDF engine, and PDFs in Close-Up

Status: **accepted 2026-11-19** (Dave: "close-up, drawn signature first,
placeholder for jpeg 2000"; the corpus: "use the PDFs in my Documents
folder for testing"). Steps 1 to 3 built 2026-11-19 (below); step 4,
text, next.

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
