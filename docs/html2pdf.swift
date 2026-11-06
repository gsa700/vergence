// html2pdf: print an HTML file to a paginated PDF with WebKit (Letter).
// usage: html2pdf in.html out.pdf
import AppKit
import WebKit

let args = CommandLine.arguments
guard args.count == 3 else { FileHandle.standardError.write("usage: html2pdf in.html out.pdf\n".data(using: .utf8)!); exit(2) }
let input = URL(fileURLWithPath: args[1])
let output = URL(fileURLWithPath: args[2])

final class Printer: NSObject, WKNavigationDelegate {
    let web: WKWebView
    let window: NSWindow
    init(frame: NSRect) {
        web = WKWebView(frame: frame)
        window = NSWindow(contentRect: frame, styleMask: [.borderless], backing: .buffered, defer: false)
        super.init()
        window.contentView = web
        web.navigationDelegate = self
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        // Let fonts and layout settle.
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { self.print() }
    }
    func print() {
        let info = NSPrintInfo()
        info.paperSize = NSSize(width: 612, height: 792)          // US Letter, points
        info.topMargin = 0; info.bottomMargin = 0; info.leftMargin = 0; info.rightMargin = 0
        info.horizontalPagination = .automatic
        info.verticalPagination = .automatic
        info.isHorizontallyCentered = false
        info.isVerticallyCentered = false
        info.jobDisposition = .save
        info.dictionary()[NSPrintInfo.AttributeKey.jobSavingURL] = output
        let op = web.printOperation(with: info)
        op.showsPrintPanel = false
        op.showsProgressPanel = false
        op.view?.frame = web.bounds
        op.runModal(for: window, delegate: self, didRun: #selector(done(_:success:context:)), contextInfo: nil)
    }
    @objc func done(_ op: NSPrintOperation, success: Bool, context: UnsafeMutableRawPointer?) {
        Swift.print(success ? "wrote \(output.path)" : "print failed")
        exit(success ? 0 : 1)
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let printer = Printer(frame: NSRect(x: 0, y: 0, width: 816, height: 1056))
printer.web.loadFileURL(input, allowingReadAccessTo: input.deletingLastPathComponent())
DispatchQueue.main.asyncAfter(deadline: .now() + 60) { Swift.print("timed out"); exit(1) }
app.run()
