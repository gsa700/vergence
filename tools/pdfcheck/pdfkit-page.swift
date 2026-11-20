// PDF milestone 1, step 9: one page drawn by Apple's PDFKit to a PNG, the
// tie-breaker when ours and pdf.js differ (pdf.js under Node gets some
// transparency wrong). On a Mac only (Control):
//   swiftc -O -o pdfkit-page tools/pdfcheck/pdfkit-page.swift
//   ./pdfkit-page file.pdf page width out.png
import AppKit
import PDFKit

let a = CommandLine.arguments
guard a.count == 5, let doc = PDFDocument(url: URL(fileURLWithPath: a[1])),
      let n = Int(a[2]), let page = doc.page(at: n - 1), let width = Double(a[3]) else {
    FileHandle.standardError.write("usage: file.pdf page width out.png\n".data(using: .utf8)!)
    exit(1)
}
let box = page.bounds(for: .cropBox)
let rotated = page.rotation % 180 != 0
let (pw, ph) = rotated ? (box.height, box.width) : (box.width, box.height)
let size = NSSize(width: width, height: (width * ph / pw).rounded())
let img = page.thumbnail(of: size, for: .cropBox)
guard let tiff = img.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff),
      let png = rep.representation(using: .png, properties: [:]) else { exit(1) }
try! png.write(to: URL(fileURLWithPath: a[4]))
