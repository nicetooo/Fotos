import Foundation
import UIKit
import WebKit

/// Bridge between Tauri WebView and native photo picker
@objc public class TauriPhotoBridge: NSObject {

    /// Shared instance
    @objc public static let shared = TauriPhotoBridge()

    /// WebView reference for sending results back
    private weak var webView: WKWebView?

    /// Database path for storing photos
    private var dbPath: String = ""

    /// Thumbnail directory
    private var thumbDir: String = ""

    private override init() {
        super.init()
    }

    /// Configure with app paths
    @objc public func configure(dbPath: String, thumbDir: String) {
        self.dbPath = dbPath
        self.thumbDir = thumbDir
    }

    /// Set the WebView reference
    @objc public func setWebView(_ webView: WKWebView) {
        self.webView = webView
    }

    /// Show photo picker
    @objc public func showPhotoPicker() {
        guard let windowScene = UIApplication.shared.connectedScenes.first as? UIWindowScene,
              let rootVC = windowScene.windows.first?.rootViewController else {
            sendErrorToWebView("Unable to find root view controller")
            return
        }

        // Check authorization first
        let status = PhotoPickerManager.shared.checkAuthorizationStatus()

        if status == 0 { // Not determined
            PhotoPickerManager.shared.requestAuthorization { [weak self] granted in
                if granted {
                    self?.presentPicker(from: rootVC)
                } else {
                    self?.sendErrorToWebView("Photo library access denied")
                }
            }
        } else if status == 2 { // Denied
            sendErrorToWebView("Photo library access denied. Please enable in Settings.")
        } else {
            presentPicker(from: rootVC)
        }
    }

    private func presentPicker(from viewController: UIViewController) {
        PhotoPickerManager.shared.onPhotosSelected = { [weak self] photos in
            self?.handleSelectedPhotos(photos)
        }

        PhotoPickerManager.shared.onPickerCancelled = { [weak self] in
            self?.sendResultToWebView(success: true, count: 0, message: "Selection cancelled")
        }

        PhotoPickerManager.shared.presentPhotoPicker(from: viewController, selectionLimit: 0)
    }

    private func handleSelectedPhotos(_ photos: [PhotoPickerManager.PhotoData]) {
        guard !photos.isEmpty else {
            sendResultToWebView(success: true, count: 0, message: "No photos selected")
            return
        }

        // Process photos in background
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }

            var processedCount = 0

            for photo in photos {
                // Save photo data to temp file for processing
                let tempDir = FileManager.default.temporaryDirectory
                let tempPath = tempDir.appendingPathComponent("\(photo.identifier).jpg")

                do {
                    try photo.imageData.write(to: tempPath)

                    // Create metadata JSON
                    var metadata: [String: Any] = [
                        "identifier": photo.identifier,
                        "filename": photo.filename,
                        "path": tempPath.path
                    ]

                    if let date = photo.creationDate {
                        let formatter = ISO8601DateFormatter()
                        metadata["date_taken"] = formatter.string(from: date)
                    }

                    if let location = photo.location {
                        metadata["lat"] = location.coordinate.latitude
                        metadata["lon"] = location.coordinate.longitude
                    }

                    // Send to WebView for processing via Tauri
                    self.sendPhotoForProcessing(metadata)
                    processedCount += 1

                } catch {
                    print("Error saving photo: \(error)")
                }
            }

            DispatchQueue.main.async {
                self.sendResultToWebView(success: true, count: processedCount, message: "Processed \(processedCount) photos")
            }
        }
    }

    private func sendPhotoForProcessing(_ metadata: [String: Any]) {
        guard let jsonData = try? JSONSerialization.data(withJSONObject: metadata),
              let jsonString = String(data: jsonData, encoding: .utf8) else {
            return
        }

        let escapedJson = jsonString.replacingOccurrences(of: "'", with: "\\'")

        DispatchQueue.main.async { [weak self] in
            let script = """
            window.__TAURI_INTERNALS__?.invoke('process_ios_photo', { photoJson: '\(escapedJson)' })
                .catch(err => console.error('Error processing photo:', err));
            """
            self?.webView?.evaluateJavaScript(script, completionHandler: nil)
        }
    }

    private func sendResultToWebView(success: Bool, count: Int, message: String) {
        DispatchQueue.main.async { [weak self] in
            let script = """
            window.dispatchEvent(new CustomEvent('ios-photo-import-complete', {
                detail: { success: \(success), count: \(count), message: '\(message)' }
            }));
            """
            self?.webView?.evaluateJavaScript(script, completionHandler: nil)
        }
    }

    private func sendErrorToWebView(_ error: String) {
        sendResultToWebView(success: false, count: 0, message: error)
    }

    /// Show Limited Library Picker
    @objc public func showLimitedLibraryPicker() {
        guard let windowScene = UIApplication.shared.connectedScenes.first as? UIWindowScene,
              let rootVC = windowScene.windows.first?.rootViewController else {
            sendErrorToWebView("Unable to find root view controller")
            return
        }

        PhotoPickerManager.shared.presentLimitedLibraryPicker(from: rootVC)
    }
}

// MARK: - WKScriptMessageHandler for receiving commands from JavaScript
extension TauriPhotoBridge: WKScriptMessageHandler {
    public func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        guard message.name == "fotosPhotoPicker" else { return }

        if let body = message.body as? [String: Any] {
            let command = body["command"] as? String ?? ""

            switch command {
            case "showPicker":
                dbPath = body["dbPath"] as? String ?? dbPath
                thumbDir = body["thumbDir"] as? String ?? thumbDir
                showPhotoPicker()

            case "showLimitedLibrary":
                showLimitedLibraryPicker()

            case "checkPermission":
                let status = PhotoPickerManager.shared.checkAuthorizationStatus()
                let script = "window.dispatchEvent(new CustomEvent('ios-permission-status', { detail: { status: \(status) } }));"
                webView?.evaluateJavaScript(script, completionHandler: nil)

            case "getAuthorizedCount":
                let count = PhotoPickerManager.shared.getAuthorizedPhotoCount()
                let script = "window.dispatchEvent(new CustomEvent('ios-authorized-count', { detail: { count: \(count) } }));"
                webView?.evaluateJavaScript(script, completionHandler: nil)

            default:
                print("Unknown command: \(command)")
            }
        }
    }
}
