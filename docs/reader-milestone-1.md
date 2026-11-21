# Reader milestone 1: documents get their own app

Status: **accepted 2026-11-20** (Dave: the name "Reader it is"; reading
only, editing in a second milestone; tabs: "tabs are nice, IF all the
documents are of a similar layout... I suppose we could just open that
tab in another window then? Ok, tab view"). Proposed the same day (Dave,
wishlist-2: "We need to have a unique window spec for when Close-Up is
displaying PDF vs images or other... Maybe we should make it its own
application?"; Claude agreed; Dave: "then let's build the pdf app").

## Why

A PDF is a document: pages, an outline, links, find, selection, later
notes and forms. A picture wants zoom, rotate, and later editing. One
window doing both is what the Mac's Preview spends much effort on; two
apps let each window be simple and right. The engine is done (PDF
milestone 1, closed 2026-11-20: vergence-pdf in its confined helper,
matching Apple's PDFKit); Close-Up's PDF view (crates/vergence-closeup
pdf.rs, about 990 lines) moves over almost as it is. Close-Up keeps
pictures, and later video.

## The steps

1. **The app: Reader** (`vergence-reader`; desktop entry GenericName "PDF
   Reader", keywords pdf, document, so the launcher finds it by "pdf"). A crate of its own, one process for
   all its windows (as Edit and Close-Up: hand.rs), its desktop entry and
   icon, the default for application/pdf (vergence-types; Close-Up's entry
   drops PDF). Close-Up given a PDF hands it over. Places in documents
   (page, zoom) kept as Close-Up keeps them now, moved over once.
2. **A document's window.** A toolbar the server's bar carries (as
   FileView's): the sidebar's switch, the page ("12 of 340", a field to
   type a page into), back and forward through pages visited (links, the
   outline, find), zoom out and in, the fit modes (width, page, actual
   size), find. The view: continuous (today's), single page, two pages
   side by side (a book's spread, the cover alone), rotate. Keys as
   PDFKit and Preview have them (Page Up/Down, Home/End, Cmd/Ctrl+
   plus and minus, Ctrl+G go to page, Ctrl+F).
3. **The sidebar.** Page thumbnails (drawn small by the helper, as
   FileView's previews), the outline (today's), and find's results with a
   line of text around each; a click goes there.
4. **Reading comfort.** Selecting and copying text (today's), Select All
   on a page, a night look (pages drawn with their light and dark
   inverted, pictures kept), the document's own title in the bar
   (Info /Title, else the file's name), the document's properties (pages,
   producer, dates, encryption) in an Info window.
5. **Opening.** Files from FileView, the desktop, Open With and the Open
   dialog; a locked PDF asks for its password in the window (today's);
   files changed on disk read again, as Edit does; several documents in
   tabs (the server's tabs, as Edit's). Each tab keeps its own view (fit,
   single or two pages, zoom, rotation): switching tabs lays its document
   out again in the same window, and a document wanting another window
   shape is dragged out of the row, or opened by "Open in New Window"
   (Dave's point: a wide layout and a paper one side by side).
6. **Its meaning.** The page area, the sidebar and the toolbar publish
   their roles and names, so agents (meaning milestone 1) and the
   magnifier follow them: the page number, the find field, the
   thumbnails as a list.
7. **Checks.** The PDF corpus opened in the app (pages drawn as before,
   no panics), the display suite on a test document (the window, the
   sidebar, two-up), tests on all four machines.

## Not in milestone 1

Editing: notes, highlights, filling forms, reordering or deleting pages,
signing (question 2). Printing (no printing system yet). Video (Close-Up's).

## Questions for Dave (answered 2026-11-20)

1. *Answered: Reader* (Folio is GNOME's Markdown notes app on Flathub;
   PageView the runner-up). **Its name.** The program stays `vergence-<name>`. Some candidates:
   **Folio** (a book's leaf, short, no one else's desktop app),
   **Reader**, **Document Viewer** (plain, GNOME's is "Document
   Viewer"), **Pages** (taken by Apple). Recommended: Folio.
2. *Answered: (a) reading only.* **Editing in this milestone?** (a) reading only, editing in a second
   milestone (recommended: the window and the reading parts first, done
   well); (b) notes and highlights now, forms and pages later; (c) all of
   it now.
3. *Answered: (a) tabs, each with its own view; drag one out for its
   own window.* **Several documents:** (a) tabs, as Edit (recommended: same as our
   other apps); (b) a window each.

## Progress

- **Step 1, the app** (2026-11-20). crates/vergence-reader: Close-Up's
  pdf.rs moved over whole (git mv), its document window taken from
  Close-Up's main.rs with the picture and text parts left behind, hand.rs
  with Reader's own socket (vergence-reader.sock), `[reader]` settings
  (the ground around the pages, style, bar, opacity, window places).
  Places in documents: ~/.local/state/vergence/reader-places, Close-Up's
  closeup-pdf-places renamed to it the first time Reader looks. Close-Up
  keeps pictures and text; given a PDF (from FileView, Open With, a drop)
  it starts Reader with it, and a window with nothing else closes. Desktop
  entries: Reader's (GenericName "PDF Reader", MimeType application/pdf,
  Keywords pdf, document), Close-Up's without PDF ("Picture Viewer"), so
  vergence-types makes Reader the default for PDFs. Its icon (an open
  book with a red ribbon, data/icons/vergence-reader.svg); install.sh and
  the launcher's own-apps order know it. Tried headless on Hubble: a
  253-page manual from the corpus opened, fit to the width; Close-Up given
  the same file handed it to the running Reader and exited. Tests on all
  four.
- **Step 2, a document's window** (2026-11-20). pdf.rs sets the pages
  out in rows (`View`): continuous (one a row, scrolled), one page at a
  time (scrolling held within the page, going on to the next past its end
  and back past its start) and two pages (a book's spread, the cover
  alone); each page's place is `spots` (its left edge from the middle, its
  row's top), and the fit modes fit a row. Turned a quarter at a time
  (`turn`, Ctrl+R and Ctrl+L): the worker turns each picture it draws,
  text, links and matches stay in the page's own points and are turned
  where drawn or hit (`turn_box`, `unturn_point`); no tiles while turned.
  The toolbar: the sidebar; Back and Forward through pages gone to by
  links, the outline, find's first match, thumbnails and the page field
  (Ctrl+[ and Ctrl+], Alt+Left and Right); previous and next with the
  page field between ("2 of 253"; Ctrl+Alt+G goes to it, Enter or Escape
  hands the keys back to the pages); zoom out, in, the width, the whole
  page, actual size; the three views; turning; Find and Open. New
  toolbar icons (vergence-widgets): FitWidth, OnePage, Continuous,
  TwoPages. Tried headless on Hubble: two pages, one page turned right.
  Tests for spreads, the one page's edges and turning.
- **Step 3, the sidebar** (2026-11-20). Pages (thumbnails, turned with
  the pages) and Contents (the outline) as before, and **Results** while a
  search is on: each match with its page and up to 40 characters either
  side on one line, cut at words (pdf.rs `snippet`, made by the worker as
  it searches); a click goes there (Back returns), the match shown is
  marked and kept in sight; typing a search with the sidebar open turns
  it to Results. Tried headless on Hubble: "lookup" in a 253-page manual,
  57 found, listed with their text.
- **Step 4, reading comfort** (2026-11-20). The night look (View > Night
  look, N): the worker draws each page with its lightness inverted and its
  hue kept (white paper a soft black, black text a soft white, a red line
  still red), the thumbnails with it; pictures go too (keeping them as
  they are needs the helper to draw them apart: later). The bar shows the
  document's own title (its Info /Title) when it is a real one, not a
  file's name ("Microsoft Word - a.docx") or "Untitled"; else the file's
  name. The Info window (Ctrl+I, the toolbar's Info): the file, folder and
  size, the pages and their paper (A4, Letter... in mm and inches), and
  what the document says about itself: title, author, subject, keywords,
  "Made with", "PDF made by", created and modified (dates made readable),
  the PDF version, encryption. vergence-pdf `Document::metadata` and
  `pdf_date`, the helper's `M` request. Select All on a page and copying
  were already in. Tried headless on Hubble.
- **Step 5, opening** (2026-11-20). Each document in a tab of the
  server's (as Edit's), each tab keeping its own view (fit, continuous /
  one page / two pages, zoom, turn, night) and its own Back and Forward;
  switching tabs lays its document out again in the same window, and the
  search goes on in it. + opens a document; a tab dragged out of the row
  (or "Move to a new window") is a window of its own, dropped on another
  Reader window it joins it (its worker then wakes that window). Files
  from the command line, a second start (handed over), Open… or a drop
  all open in tabs; one already open comes to the front. Every 2 s the
  open files are looked at (modified time and length, as Edit does): one
  changed on disk is read again, its zoom, view, turn, night look and
  page put back. A locked PDF still asks for its password in its tab.
  Tried headless on Hubble: two tabs, two views kept through switching,
  a file replaced on disk read again (20 pages to 4, its own title in the
  tab), a second start handed over.
