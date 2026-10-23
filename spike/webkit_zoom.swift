// Temporary spike (#268): WKWebView under pageZoom, the property wry sets for Webview::set_zoom.
import AppKit
import WebKit

let script = try! String(contentsOfFile: CommandLine.arguments[1], encoding: .utf8)
let page = "<!doctype html><body style=\"margin:0\"><div id=\"box\" style=\"width:100px;height:10px\"></div></body>"

final class Spike: NSObject, WKNavigationDelegate {
  let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 800, height: 600), styleMask: [.titled], backing: .buffered, defer: false)
  let view = WKWebView(frame: NSRect(x: 0, y: 0, width: 800, height: 600))
  var results: [String: Any] = [:]

  func start() {
    window.contentView = view
    window.makeKeyAndOrderFront(nil)
    view.navigationDelegate = self
    view.loadHTMLString(page, baseURL: nil)
  }

  func measure(_ label: String, then: @escaping () -> Void) {
    view.evaluateJavaScript(script) { value, error in
      var row: [String: Any] = ["error": error.map { "\($0)" } ?? NSNull()]
      if let text = value as? String, let obj = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any] {
        row = obj
      }
      row["view"] = [self.view.frame.width, self.view.frame.height]
      row["backingScale"] = self.window.backingScaleFactor
      row["pageZoom"] = self.view.pageZoom
      self.results[label] = row
      then()
    }
  }

  func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
      self.measure("zoom_1") {
        self.view.pageZoom = 1.5
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) {
          self.measure("zoom_1.5") {
            let data = try! JSONSerialization.data(withJSONObject: self.results, options: [.prettyPrinted, .sortedKeys])
            print(String(data: data, encoding: .utf8)!)
            exit(0)
          }
        }
      }
    }
  }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let spike = Spike()
spike.start()
DispatchQueue.main.asyncAfter(deadline: .now() + 30) {
  print("timeout", spike.results)
  exit(1)
}
app.run()
