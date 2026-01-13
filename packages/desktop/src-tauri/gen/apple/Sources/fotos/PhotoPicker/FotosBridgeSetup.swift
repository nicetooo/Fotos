import UIKit
import WebKit

/// Helper class to set up the Swift-JavaScript bridge
@objc public class FotosBridgeSetup: NSObject {

    @objc public static let shared = FotosBridgeSetup()

    private var setupComplete = false
    private var retryCount = 0
    private let maxRetries = 30

    private override init() {
        super.init()
    }

    /// Start watching for WebView and set up bridge
    @objc public func start() {
        guard !setupComplete else { return }
        print("[FotosBridge] Starting to watch for WebView...")
        attemptSetup()
    }

    private func attemptSetup() {
        guard !setupComplete && retryCount < maxRetries else {
            if !setupComplete {
                print("[FotosBridge] Failed to find WebView after \(maxRetries) attempts")
            }
            return
        }
        retryCount += 1

        // Find the WKWebView in the view hierarchy
        guard let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene,
              let window = scene.windows.first,
              let webView = findWebView(in: window) else {
            // Try again later
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in
                self?.attemptSetup()
            }
            return
        }

        print("[FotosBridge] Found WebView on attempt \(retryCount)")
        setupBridge(webView: webView)
    }

    private func setupBridge(webView: WKWebView) {
        guard !setupComplete else { return }
        setupComplete = true

        // Set the WebView reference in TauriPhotoBridge
        TauriPhotoBridge.shared.setWebView(webView)

        // Add script message handler
        let contentController = webView.configuration.userContentController
        contentController.add(TauriPhotoBridge.shared, name: "fotosPhotoPicker")

        print("[FotosBridge] Bridge setup complete")

        // Inject notification that bridge is ready
        let testScript = """
        console.log('[Fotos] Swift bridge initialized');
        window.__FOTOS_BRIDGE_READY__ = true;
        window.dispatchEvent(new CustomEvent('fotos-bridge-ready', { detail: {} }));
        """
        webView.evaluateJavaScript(testScript, completionHandler: nil)
    }

    private func findWebView(in view: UIView) -> WKWebView? {
        if let webView = view as? WKWebView {
            return webView
        }
        for subview in view.subviews {
            if let found = findWebView(in: subview) {
                return found
            }
        }
        return nil
    }
}

// Use Swift's module initialization to start the bridge setup
// This struct's static property will be initialized when the module loads
private enum BridgeAutoStart {
    static let start: Void = {
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
            FotosBridgeSetup.shared.start()
        }
    }()
}

// Reference the static property to force initialization
// This happens at app launch due to the @_cdecl attribute
@_cdecl("FotosBridgeInit")
public func FotosBridgeInit() {
    _ = BridgeAutoStart.start
}
