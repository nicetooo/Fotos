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

        // Status: 0=notDetermined, 1=restricted, 2=denied, 3=authorized, 4=limited
        switch status {
        case 0: // Not determined - request permission
            PhotoPickerManager.shared.requestAuthorization { [weak self] granted in
                if granted {
                    // After granting, check the actual status and handle accordingly
                    let newStatus = PhotoPickerManager.shared.checkAuthorizationStatus()
                    if newStatus == 4 { // Limited
                        self?.showLimitedAccessAlert(from: rootVC)
                    } else {
                        self?.presentPicker(from: rootVC)
                    }
                } else {
                    self?.sendErrorToWebView("Photo library access denied")
                }
            }
        case 1: // Restricted
            sendErrorToWebView("Photo library access is restricted on this device")
        case 2: // Denied
            showDeniedAccessAlert(from: rootVC)
        case 3: // Full access - just show picker
            presentPicker(from: rootVC)
        case 4: // Limited access - show options
            showLimitedAccessAlert(from: rootVC)
        default:
            presentPicker(from: rootVC)
        }
    }

    /// Show alert for limited access - let user choose more photos or go to settings
    private func showLimitedAccessAlert(from viewController: UIViewController) {
        let alert = UIAlertController(
            title: "照片访问受限",
            message: "您只授权了部分照片访问权限。您可以选择更多照片或在设置中允许访问所有照片。",
            preferredStyle: .actionSheet
        )

        // Option 1: Select more photos to authorize
        alert.addAction(UIAlertAction(title: "选择更多照片", style: .default) { [weak self] _ in
            if #available(iOS 14, *) {
                PhotoPickerManager.shared.presentLimitedLibraryPicker(from: viewController)
            }
        })

        // Option 2: Continue with current selection (use PHPicker which works regardless of authorization)
        alert.addAction(UIAlertAction(title: "从相册选择", style: .default) { [weak self] _ in
            self?.presentPicker(from: viewController)
        })

        // Option 3: Go to Settings to grant full access
        alert.addAction(UIAlertAction(title: "前往设置", style: .default) { _ in
            if let settingsUrl = URL(string: UIApplication.openSettingsURLString) {
                UIApplication.shared.open(settingsUrl)
            }
        })

        alert.addAction(UIAlertAction(title: "取消", style: .cancel))

        // For iPad support
        if let popoverController = alert.popoverPresentationController {
            popoverController.sourceView = viewController.view
            popoverController.sourceRect = CGRect(x: viewController.view.bounds.midX, y: viewController.view.bounds.midY, width: 0, height: 0)
            popoverController.permittedArrowDirections = []
        }

        viewController.present(alert, animated: true)
    }

    /// Show alert for denied access - guide user to settings
    private func showDeniedAccessAlert(from viewController: UIViewController) {
        let alert = UIAlertController(
            title: "无法访问照片",
            message: "请在设置中允许 Fotos 访问您的照片库。",
            preferredStyle: .alert
        )

        alert.addAction(UIAlertAction(title: "前往设置", style: .default) { _ in
            if let settingsUrl = URL(string: UIApplication.openSettingsURLString) {
                UIApplication.shared.open(settingsUrl)
            }
        })

        alert.addAction(UIAlertAction(title: "取消", style: .cancel) { [weak self] _ in
            self?.sendErrorToWebView("Photo library access denied")
        })

        viewController.present(alert, animated: true)
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

    /// Request permission and import all authorized photos
    /// This is the main entry point for iOS import - handles both full and limited access
    @objc public func requestAndImportPhotos() {
        let status = PhotoPickerManager.shared.checkAuthorizationStatus()

        // Status: 0=notDetermined, 1=restricted, 2=denied, 3=authorized, 4=limited
        switch status {
        case 0: // Not determined - request permission
            PhotoPickerManager.shared.requestAuthorization { [weak self] granted in
                if granted {
                    // Check what level of access we got
                    let newStatus = PhotoPickerManager.shared.checkAuthorizationStatus()
                    self?.handlePermissionResult(status: newStatus)
                } else {
                    self?.sendPermissionDenied()
                }
            }
        case 1: // Restricted
            sendErrorToWebView("Photo library access is restricted on this device")
        case 2: // Denied
            sendPermissionDenied()
        case 3, 4: // Authorized or Limited - import all accessible photos
            importAllAuthorizedPhotos()
        default:
            sendErrorToWebView("Unknown permission status")
        }
    }

    /// Auto-sync photos on app launch if user has granted full access
    /// This silently imports any new photos since last launch
    @objc public func syncPhotosIfFullAccess() {
        let status = PhotoPickerManager.shared.checkAuthorizationStatus()

        // Send permission status to frontend
        DispatchQueue.main.async { [weak self] in
            let script = "window.dispatchEvent(new CustomEvent('ios-permission-status', { detail: { status: \(status) } }));"
            self?.webView?.evaluateJavaScript(script, completionHandler: nil)
        }

        // Only auto-sync if user has granted full access (status == 3)
        // For limited access, user should manually trigger import via + button
        if status == 3 {
            print("[PhotoPicker] Full access granted, auto-syncing photos...")
            importAllAuthorizedPhotos()
        }
    }

    private func handlePermissionResult(status: Int) {
        switch status {
        case 3: // Full access
            DispatchQueue.main.async { [weak self] in
                let script = "window.dispatchEvent(new CustomEvent('ios-permission-granted', { detail: { type: 'full' } }));"
                self?.webView?.evaluateJavaScript(script, completionHandler: nil)
            }
            importAllAuthorizedPhotos()
        case 4: // Limited access
            DispatchQueue.main.async { [weak self] in
                let script = "window.dispatchEvent(new CustomEvent('ios-permission-granted', { detail: { type: 'limited' } }));"
                self?.webView?.evaluateJavaScript(script, completionHandler: nil)
            }
            importAllAuthorizedPhotos()
        default:
            sendPermissionDenied()
        }
    }

    private func sendPermissionDenied() {
        DispatchQueue.main.async { [weak self] in
            let script = """
            window.dispatchEvent(new CustomEvent('ios-permission-denied', {
                detail: { message: 'Photo library access denied. Please enable in Settings.' }
            }));
            """
            self?.webView?.evaluateJavaScript(script, completionHandler: nil)
        }
    }

    /// Import all authorized photos
    @objc public func importAllAuthorizedPhotos() {
        // Send start event
        DispatchQueue.main.async { [weak self] in
            let script = "window.dispatchEvent(new CustomEvent('ios-import-started', { detail: {} }));"
            self?.webView?.evaluateJavaScript(script, completionHandler: nil)
        }

        // Export all photos to temp files
        PhotoPickerManager.shared.exportAllAuthorizedPhotos(
            progressCallback: { [weak self] current, total in
                // Send export progress
                DispatchQueue.main.async {
                    let script = """
                    window.dispatchEvent(new CustomEvent('ios-export-progress', {
                        detail: { current: \(current), total: \(total), phase: 'exporting' }
                    }));
                    """
                    self?.webView?.evaluateJavaScript(script, completionHandler: nil)
                }
            },
            completion: { [weak self] paths in
                guard let self = self else { return }

                if paths.isEmpty {
                    self.sendResultToWebView(success: true, count: 0, message: "No photos to import")
                    return
                }

                // Send paths to JavaScript for Tauri import
                // Process photos one by one via Tauri invoke
                DispatchQueue.main.async {
                    let pathsJson = paths.map { "\"\($0)\"" }.joined(separator: ",")
                    let script = """
                    (async function() {
                        const paths = [\(pathsJson)];
                        const total = paths.length;
                        let success = 0;
                        let duplicates = 0;
                        let failure = 0;

                        for (let i = 0; i < paths.length; i++) {
                            try {
                                const result = await window.__TAURI_INTERNALS__?.invoke('import_photos', {
                                    rootPath: 'file://' + paths[i],
                                    dbPath: '\(self.dbPath)',
                                    thumbDir: '\(self.thumbDir)'
                                });
                                if (result) {
                                    success += result.success || 0;
                                    duplicates += result.duplicates || 0;
                                    failure += result.failure || 0;
                                }
                            } catch (err) {
                                console.error('Import error:', err);
                                failure++;
                            }

                            // Send progress
                            window.dispatchEvent(new CustomEvent('ios-import-progress', {
                                detail: {
                                    current: i + 1,
                                    total: total,
                                    success: success,
                                    duplicates: duplicates,
                                    failure: failure,
                                    phase: 'importing'
                                }
                            }));
                        }

                        // Send completion
                        window.dispatchEvent(new CustomEvent('ios-import-complete', {
                            detail: { success: success, duplicates: duplicates, failure: failure, total: total }
                        }));

                        // Reload photos
                        window.dispatchEvent(new CustomEvent('reload-photos', { detail: {} }));
                    })();
                    """
                    self.webView?.evaluateJavaScript(script, completionHandler: nil)
                }
            }
        )
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

            case "importAllAuthorized":
                dbPath = body["dbPath"] as? String ?? dbPath
                thumbDir = body["thumbDir"] as? String ?? thumbDir
                importAllAuthorizedPhotos()

            case "requestAndImport":
                dbPath = body["dbPath"] as? String ?? dbPath
                thumbDir = body["thumbDir"] as? String ?? thumbDir
                requestAndImportPhotos()

            case "syncIfFullAccess":
                // Auto-sync photos on app launch if user has granted full access
                dbPath = body["dbPath"] as? String ?? dbPath
                thumbDir = body["thumbDir"] as? String ?? thumbDir
                syncPhotosIfFullAccess()

            default:
                print("Unknown command: \(command)")
            }
        }
    }
}
