//! The table of file types: our own, written for Vergence (Dave,
//! 2026-11-15: not the system's shared-mime-info, whose data is GPL). Type
//! names are the freedesktop ones (what desktop entries' MimeType lines and
//! mimeapps.list use), so other programs' entries match ours.
//!
//! Each row: the type, what it is called, its generic icon (the freedesktop
//! icon naming spec's), the name patterns (`*.ext`, matched without regard
//! to case, or a whole file name, matched exactly), and the type it is a
//! kind of, if any besides the defaults (every `text/*` is a kind of
//! `text/plain`, and everything but folders a kind of
//! `application/octet-stream`).

pub struct Row {
    pub mime: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    pub globs: &'static [&'static str],
    pub parent: Option<&'static str>,
}

const fn r(mime: &'static str, name: &'static str, icon: &'static str, globs: &'static [&'static str], parent: Option<&'static str>) -> Row {
    Row { mime, name, icon, globs, parent }
}

const TEXT: &str = "text-x-generic";
const CODE: &str = "text-x-script";
const IMAGE: &str = "image-x-generic";
const AUDIO: &str = "audio-x-generic";
const VIDEO: &str = "video-x-generic";
const DOC: &str = "x-office-document";
const SHEET: &str = "x-office-spreadsheet";
const SLIDES: &str = "x-office-presentation";
const PACKAGE: &str = "package-x-generic";
const EXEC: &str = "application-x-executable";
const FONT: &str = "font-x-generic";
const PLAIN: Option<&str> = Some("text/plain");

pub static ROWS: &[Row] = &[
    // Folders and the catch-all.
    r("inode/directory", "Folder", "folder", &[], None),
    r("application/octet-stream", "Binary file", EXEC, &[], None),
    // Text.
    r("text/plain", "Plain text", TEXT, &["*.txt", "*.text", "*.conf", "*.cfg", "README", "INSTALL", "COPYING", "LICENSE", "NEWS", "AUTHORS"], None),
    r("text/x-log", "Log", TEXT, &["*.log"], None),
    r("text/markdown", "Markdown", TEXT, &["*.md", "*.markdown", "*.mkd"], None),
    r("text/csv", "Comma-separated values", SHEET, &["*.csv"], None),
    r("text/tab-separated-values", "Tab-separated values", SHEET, &["*.tsv"], None),
    r("text/html", "Web page", "text-html", &["*.html", "*.htm"], None),
    r("application/xhtml+xml", "Web page (XHTML)", "text-html", &["*.xhtml"], Some("application/xml")),
    r("text/css", "Style sheet", CODE, &["*.css"], None),
    r("application/xml", "XML", CODE, &["*.xml", "*.xsd", "*.xsl", "*.plist"], PLAIN),
    r("application/json", "JSON", CODE, &["*.json"], PLAIN),
    r("application/toml", "TOML", CODE, &["*.toml", "Cargo.lock"], PLAIN),
    r("application/yaml", "YAML", CODE, &["*.yaml", "*.yml"], PLAIN),
    r("text/x-ini", "Settings (INI)", TEXT, &["*.ini"], None),
    r("application/x-desktop", "Desktop entry", TEXT, &["*.desktop"], PLAIN),
    r("text/calendar", "Calendar", "x-office-calendar", &["*.ics"], None),
    r("text/vcard", "Contact card", "x-office-address-book", &["*.vcf", "*.vcard"], None),
    r("application/x-subrip", "Subtitles", TEXT, &["*.srt"], PLAIN),
    r("application/rtf", "Rich text", DOC, &["*.rtf"], PLAIN),
    r("text/x-patch", "Patch", CODE, &["*.patch", "*.diff"], None),
    r("text/x-tex", "TeX", CODE, &["*.tex", "*.sty", "*.cls"], None),
    // Source code.
    r("text/rust", "Rust source", CODE, &["*.rs"], None),
    r("text/x-csrc", "C source", CODE, &["*.c"], None),
    r("text/x-chdr", "C header", CODE, &["*.h"], None),
    r("text/x-c++src", "C++ source", CODE, &["*.cpp", "*.cc", "*.cxx", "*.c++"], None),
    r("text/x-c++hdr", "C++ header", CODE, &["*.hpp", "*.hh", "*.hxx"], None),
    r("text/x-csharp", "C# source", CODE, &["*.cs"], None),
    r("text/x-java", "Java source", CODE, &["*.java"], None),
    r("text/x-python", "Python script", CODE, &["*.py", "*.pyw"], None),
    r("text/javascript", "JavaScript", CODE, &["*.js", "*.mjs", "*.cjs"], None),
    r("text/x-go", "Go source", CODE, &["*.go"], None),
    r("text/x-swift", "Swift source", CODE, &["*.swift"], None),
    r("text/x-lua", "Lua script", CODE, &["*.lua"], None),
    r("application/x-ruby", "Ruby script", CODE, &["*.rb"], PLAIN),
    r("application/x-perl", "Perl script", CODE, &["*.pl", "*.pm"], PLAIN),
    r("application/x-shellscript", "Shell script", CODE, &["*.sh", "*.bash", "*.zsh", "*.ksh"], PLAIN),
    r("application/sql", "SQL", CODE, &["*.sql"], PLAIN),
    r("text/x-makefile", "Makefile", CODE, &["Makefile", "makefile", "GNUmakefile", "*.mk"], None),
    r("text/x-cmake", "CMake", CODE, &["CMakeLists.txt", "*.cmake"], None),
    // Pictures.
    r("image/png", "PNG picture", IMAGE, &["*.png"], None),
    r("image/jpeg", "JPEG picture", IMAGE, &["*.jpg", "*.jpeg", "*.jpe", "*.jfif"], None),
    r("image/gif", "GIF picture", IMAGE, &["*.gif"], None),
    r("image/webp", "WebP picture", IMAGE, &["*.webp"], None),
    r("image/bmp", "BMP picture", IMAGE, &["*.bmp"], None),
    r("image/svg+xml", "SVG drawing", IMAGE, &["*.svg"], Some("application/xml")),
    r("image/svg+xml-compressed", "SVG drawing (compressed)", IMAGE, &["*.svgz"], Some("application/gzip")),
    r("image/tiff", "TIFF picture", IMAGE, &["*.tif", "*.tiff"], None),
    r("image/heif", "HEIF picture", IMAGE, &["*.heic", "*.heif"], None),
    r("image/avif", "AVIF picture", IMAGE, &["*.avif"], None),
    r("image/jxl", "JPEG XL picture", IMAGE, &["*.jxl"], None),
    r("image/vnd.microsoft.icon", "Windows icon", IMAGE, &["*.ico"], None),
    r("image/x-portable-pixmap", "PPM picture", IMAGE, &["*.ppm", "*.pnm", "*.pgm", "*.pbm"], None),
    r("image/vnd.djvu", "DjVu document", DOC, &["*.djvu", "*.djv"], None),
    // Sound.
    r("audio/mpeg", "MP3 audio", AUDIO, &["*.mp3"], None),
    r("audio/flac", "FLAC audio", AUDIO, &["*.flac"], None),
    r("audio/ogg", "Ogg audio", AUDIO, &["*.ogg", "*.oga"], None),
    r("audio/x-opus+ogg", "Opus audio", AUDIO, &["*.opus"], Some("audio/ogg")),
    r("audio/x-wav", "WAV audio", AUDIO, &["*.wav"], None),
    r("audio/aac", "AAC audio", AUDIO, &["*.aac"], None),
    r("audio/mp4", "MPEG-4 audio", AUDIO, &["*.m4a", "*.m4b"], None),
    r("audio/x-aiff", "AIFF audio", AUDIO, &["*.aiff", "*.aif", "*.aifc"], None),
    r("audio/midi", "MIDI", AUDIO, &["*.mid", "*.midi"], None),
    // Video.
    r("video/mp4", "MPEG-4 video", VIDEO, &["*.mp4", "*.m4v"], None),
    r("video/x-matroska", "Matroska video", VIDEO, &["*.mkv"], None),
    r("video/webm", "WebM video", VIDEO, &["*.webm"], None),
    r("video/quicktime", "QuickTime video", VIDEO, &["*.mov", "*.qt"], None),
    r("video/x-msvideo", "AVI video", VIDEO, &["*.avi"], None),
    r("video/ogg", "Ogg video", VIDEO, &["*.ogv"], None),
    r("video/mpeg", "MPEG video", VIDEO, &["*.mpg", "*.mpeg"], None),
    r("video/mp2t", "MPEG transport stream", VIDEO, &["*.m2ts", "*.mts"], None),
    // Documents.
    r("application/pdf", "PDF document", DOC, &["*.pdf"], None),
    r("application/postscript", "PostScript document", DOC, &["*.ps", "*.eps"], None),
    r("application/epub+zip", "EPUB book", DOC, &["*.epub"], Some("application/zip")),
    r("application/vnd.oasis.opendocument.text", "OpenDocument text", DOC, &["*.odt"], Some("application/zip")),
    r("application/vnd.oasis.opendocument.spreadsheet", "OpenDocument spreadsheet", SHEET, &["*.ods"], Some("application/zip")),
    r("application/vnd.oasis.opendocument.presentation", "OpenDocument presentation", SLIDES, &["*.odp"], Some("application/zip")),
    r("application/vnd.oasis.opendocument.graphics", "OpenDocument drawing", IMAGE, &["*.odg"], Some("application/zip")),
    r("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "Word document", DOC, &["*.docx"], Some("application/zip")),
    r("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "Excel spreadsheet", SHEET, &["*.xlsx"], Some("application/zip")),
    r("application/vnd.openxmlformats-officedocument.presentationml.presentation", "PowerPoint presentation", SLIDES, &["*.pptx"], Some("application/zip")),
    r("application/msword", "Word document (old)", DOC, &["*.doc"], None),
    r("application/vnd.ms-excel", "Excel spreadsheet (old)", SHEET, &["*.xls"], None),
    r("application/vnd.ms-powerpoint", "PowerPoint presentation (old)", SLIDES, &["*.ppt"], None),
    // Archives and disk images.
    r("application/zip", "Zip archive", PACKAGE, &["*.zip"], None),
    r("application/gzip", "Gzip file", PACKAGE, &["*.gz"], None),
    r("application/x-tar", "Tar archive", PACKAGE, &["*.tar"], None),
    r("application/x-compressed-tar", "Tar archive (gzip)", PACKAGE, &["*.tar.gz", "*.tgz"], Some("application/gzip")),
    r("application/x-xz", "XZ file", PACKAGE, &["*.xz"], None),
    r("application/x-xz-compressed-tar", "Tar archive (xz)", PACKAGE, &["*.tar.xz", "*.txz"], Some("application/x-xz")),
    r("application/zstd", "Zstandard file", PACKAGE, &["*.zst"], None),
    r("application/x-zstd-compressed-tar", "Tar archive (zstd)", PACKAGE, &["*.tar.zst", "*.tzst"], Some("application/zstd")),
    r("application/x-bzip2", "Bzip2 file", PACKAGE, &["*.bz2"], None),
    r("application/x-bzip-compressed-tar", "Tar archive (bzip2)", PACKAGE, &["*.tar.bz2", "*.tbz2"], Some("application/x-bzip2")),
    r("application/x-lz4", "LZ4 file", PACKAGE, &["*.lz4"], None),
    r("application/x-7z-compressed", "7-Zip archive", PACKAGE, &["*.7z"], None),
    r("application/vnd.rar", "RAR archive", PACKAGE, &["*.rar"], None),
    r("application/java-archive", "Java archive", PACKAGE, &["*.jar"], Some("application/zip")),
    r("application/vnd.android.package-archive", "Android package", PACKAGE, &["*.apk"], Some("application/zip")),
    r("application/vnd.debian.binary-package", "Debian package", PACKAGE, &["*.deb"], None),
    r("application/x-rpm", "RPM package", PACKAGE, &["*.rpm"], None),
    r("application/x-cd-image", "Disc image", "media-optical", &["*.iso"], None),
    r("application/x-raw-disk-image", "Disk image", "drive-harddisk", &["*.img"], None),
    r("application/x-apple-diskimage", "Apple disk image", "drive-harddisk", &["*.dmg"], None),
    // Programs, fonts and the rest.
    r("application/x-executable", "Program", EXEC, &[], None),
    r("application/x-sharedlib", "Shared library", EXEC, &["*.so"], None),
    r("application/x-object", "Object code", EXEC, &["*.o"], None),
    r("application/x-ms-dos-executable", "Windows program", EXEC, &["*.exe"], None),
    r("application/x-msi", "Windows installer", PACKAGE, &["*.msi"], None),
    r("application/x-sqlite3", "SQLite database", "x-office-spreadsheet", &["*.sqlite", "*.sqlite3"], None),
    r("application/x-bittorrent", "Torrent", "application-x-bittorrent", &["*.torrent"], None),
    r("font/ttf", "TrueType font", FONT, &["*.ttf"], None),
    r("font/otf", "OpenType font", FONT, &["*.otf"], None),
    r("font/woff", "Web font", FONT, &["*.woff"], None),
    r("font/woff2", "Web font", FONT, &["*.woff2"], None),
    r("application/x-trash", "Backup file", TEXT, &["*~", "*.bak", "*.old", "*.orig"], None),
];

/// Other names programs use for the types above (desktop entries written
/// years apart do not agree), mapped to the name this table uses.
pub static ALIASES: &[(&str, &str)] = &[
    ("image/jpg", "image/jpeg"),
    ("image/pjpeg", "image/jpeg"),
    ("image/x-ms-bmp", "image/bmp"),
    ("image/x-bmp", "image/bmp"),
    ("image/heic", "image/heif"),
    ("image/x-icon", "image/vnd.microsoft.icon"),
    ("image/svg", "image/svg+xml"),
    ("application/x-pdf", "application/pdf"),
    ("text/x-markdown", "text/markdown"),
    ("audio/x-flac", "audio/flac"),
    ("audio/mp3", "audio/mpeg"),
    ("audio/x-mp3", "audio/mpeg"),
    ("audio/x-mpeg", "audio/mpeg"),
    ("audio/wav", "audio/x-wav"),
    ("audio/vnd.wave", "audio/x-wav"),
    ("audio/x-m4a", "audio/mp4"),
    ("audio/x-midi", "audio/midi"),
    ("audio/x-vorbis+ogg", "audio/ogg"),
    ("application/ogg", "audio/ogg"),
    ("video/x-ogm+ogg", "video/ogg"),
    ("application/x-gzip", "application/gzip"),
    ("application/x-zip-compressed", "application/zip"),
    ("application/x-zip", "application/zip"),
    ("application/x-zstd", "application/zstd"),
    ("application/x-bzip", "application/x-bzip2"),
    ("application/x-rar", "application/vnd.rar"),
    ("application/x-rar-compressed", "application/vnd.rar"),
    ("application/vnd.efi.iso", "application/x-cd-image"),
    ("application/x-iso9660-image", "application/x-cd-image"),
    ("text/x-python3", "text/x-python"),
    ("application/x-python", "text/x-python"),
    ("text/x-rust", "text/rust"),
    ("text/x-sh", "application/x-shellscript"),
    ("application/x-sh", "application/x-shellscript"),
    ("application/javascript", "text/javascript"),
    ("application/x-javascript", "text/javascript"),
    ("application/x-yaml", "application/yaml"),
    ("text/yaml", "application/yaml"),
    ("text/x-yaml", "application/yaml"),
    ("application/x-toml", "application/toml"),
    ("text/xml", "application/xml"),
    ("text/x-csv", "text/csv"),
    ("text/x-c", "text/x-csrc"),
    ("text/x-c++", "text/x-c++src"),
    ("text/x-vcard", "text/vcard"),
    ("text/rtf", "application/rtf"),
    ("application/x-font-ttf", "font/ttf"),
    ("application/x-font-otf", "font/otf"),
    ("application/font-woff", "font/woff"),
    ("application/x-sqlite", "application/x-sqlite3"),
    ("application/vnd.sqlite3", "application/x-sqlite3"),
    ("application/x-msdownload", "application/x-ms-dos-executable"),
    ("application/x-pie-executable", "application/x-executable"),
];
