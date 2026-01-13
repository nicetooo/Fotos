import Foundation
import PhotosUI
import UIKit

/// Manages photo library access and photo picking for iOS
@objc public class PhotoPickerManager: NSObject {

    /// Shared instance
    @objc public static let shared = PhotoPickerManager()

    /// Callback when photos are selected
    public var onPhotosSelected: (([PhotoData]) -> Void)?

    /// Callback when picker is cancelled
    public var onPickerCancelled: (() -> Void)?

    private override init() {
        super.init()
    }

    /// Photo data structure
    @objc public class PhotoData: NSObject {
        @objc public let identifier: String
        @objc public let imageData: Data
        @objc public let filename: String
        @objc public let creationDate: Date?
        @objc public let location: CLLocation?

        init(identifier: String, imageData: Data, filename: String, creationDate: Date?, location: CLLocation?) {
            self.identifier = identifier
            self.imageData = imageData
            self.filename = filename
            self.creationDate = creationDate
            self.location = location
        }
    }

    /// Request photo library authorization
    @objc public func requestAuthorization(completion: @escaping (Bool) -> Void) {
        if #available(iOS 14, *) {
            PHPhotoLibrary.requestAuthorization(for: .readWrite) { status in
                DispatchQueue.main.async {
                    completion(status == .authorized || status == .limited)
                }
            }
        } else {
            PHPhotoLibrary.requestAuthorization { status in
                DispatchQueue.main.async {
                    completion(status == .authorized)
                }
            }
        }
    }

    /// Check current authorization status
    @objc public func checkAuthorizationStatus() -> Int {
        if #available(iOS 14, *) {
            let status = PHPhotoLibrary.authorizationStatus(for: .readWrite)
            switch status {
            case .notDetermined: return 0
            case .restricted: return 1
            case .denied: return 2
            case .authorized: return 3
            case .limited: return 4
            @unknown default: return 0
            }
        } else {
            let status = PHPhotoLibrary.authorizationStatus()
            switch status {
            case .notDetermined: return 0
            case .restricted: return 1
            case .denied: return 2
            case .authorized: return 3
            @unknown default: return 0
            }
        }
    }

    /// Present photo picker
    @objc public func presentPhotoPicker(from viewController: UIViewController, selectionLimit: Int = 0) {
        if #available(iOS 14, *) {
            var config = PHPickerConfiguration(photoLibrary: .shared())
            config.selectionLimit = selectionLimit // 0 = unlimited
            config.filter = .images
            config.preferredAssetRepresentationMode = .current

            let picker = PHPickerViewController(configuration: config)
            picker.delegate = self
            viewController.present(picker, animated: true)
        } else {
            // Fallback for iOS 13
            let picker = UIImagePickerController()
            picker.sourceType = .photoLibrary
            picker.delegate = self
            viewController.present(picker, animated: true)
        }
    }

    /// Present Limited Library Picker (iOS 14+)
    /// Shows photos user has already authorized and allows selecting more
    @objc public func presentLimitedLibraryPicker(from viewController: UIViewController) {
        if #available(iOS 14, *) {
            PHPhotoLibrary.shared().presentLimitedLibraryPicker(from: viewController)
        }
    }

    /// Get count of authorized photos
    @objc public func getAuthorizedPhotoCount() -> Int {
        let fetchOptions = PHFetchOptions()
        fetchOptions.includeHiddenAssets = false
        let results = PHAsset.fetchAssets(with: .image, options: fetchOptions)
        return results.count
    }

    /// Get photos from the library (for full access mode)
    @objc public func fetchAllPhotos(limit: Int = 100, completion: @escaping ([PhotoData]) -> Void) {
        let fetchOptions = PHFetchOptions()
        fetchOptions.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
        fetchOptions.fetchLimit = limit

        let results = PHAsset.fetchAssets(with: .image, options: fetchOptions)

        var photos: [PhotoData] = []
        let group = DispatchGroup()

        results.enumerateObjects { asset, _, _ in
            group.enter()
            self.loadPhotoData(from: asset) { photoData in
                if let photoData = photoData {
                    photos.append(photoData)
                }
                group.leave()
            }
        }

        group.notify(queue: .main) {
            completion(photos)
        }
    }

    /// Export all authorized photos to temp files and return their paths
    /// This is the main function for "import all authorized photos" feature
    @objc public func exportAllAuthorizedPhotos(
        progressCallback: @escaping (Int, Int) -> Void,
        completion: @escaping ([String]) -> Void
    ) {
        let fetchOptions = PHFetchOptions()
        fetchOptions.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
        fetchOptions.includeHiddenAssets = false

        let results = PHAsset.fetchAssets(with: .image, options: fetchOptions)
        let total = results.count

        if total == 0 {
            completion([])
            return
        }

        var exportedPaths: [String] = []
        let queue = DispatchQueue(label: "photo.export", qos: .userInitiated)
        let group = DispatchGroup()

        // Get cache directory for temp files
        let cacheDir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first!

        var processed = 0
        let lock = NSLock()

        results.enumerateObjects { asset, index, _ in
            group.enter()

            let options = PHImageRequestOptions()
            options.deliveryMode = .highQualityFormat
            options.isNetworkAccessAllowed = true
            options.isSynchronous = false

            PHImageManager.default().requestImageDataAndOrientation(for: asset, options: options) { data, uti, _, info in
                defer {
                    lock.lock()
                    processed += 1
                    let current = processed
                    lock.unlock()

                    DispatchQueue.main.async {
                        progressCallback(current, total)
                    }
                    group.leave()
                }

                guard let imageData = data else { return }

                // Determine file extension
                let resources = PHAssetResource.assetResources(for: asset)
                var filename = resources.first?.originalFilename ?? "photo_\(asset.localIdentifier)"

                // Sanitize filename (remove invalid characters)
                filename = filename.replacingOccurrences(of: "/", with: "_")

                let tempPath = cacheDir.appendingPathComponent(filename)

                do {
                    try imageData.write(to: tempPath)
                    lock.lock()
                    exportedPaths.append(tempPath.path)
                    lock.unlock()
                } catch {
                    print("[PhotoPicker] Failed to write temp file: \(error)")
                }
            }
        }

        group.notify(queue: .main) {
            completion(exportedPaths)
        }
    }

    /// Load photo data from PHAsset
    private func loadPhotoData(from asset: PHAsset, completion: @escaping (PhotoData?) -> Void) {
        let options = PHImageRequestOptions()
        options.deliveryMode = .highQualityFormat
        options.isNetworkAccessAllowed = true
        options.isSynchronous = false

        PHImageManager.default().requestImageDataAndOrientation(for: asset, options: options) { data, _, _, _ in
            guard let imageData = data else {
                completion(nil)
                return
            }

            let resources = PHAssetResource.assetResources(for: asset)
            let filename = resources.first?.originalFilename ?? "photo_\(asset.localIdentifier).jpg"

            let photoData = PhotoData(
                identifier: asset.localIdentifier,
                imageData: imageData,
                filename: filename,
                creationDate: asset.creationDate,
                location: asset.location
            )

            completion(photoData)
        }
    }
}

// MARK: - PHPickerViewControllerDelegate
@available(iOS 14, *)
extension PhotoPickerManager: PHPickerViewControllerDelegate {
    public func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        picker.dismiss(animated: true)

        if results.isEmpty {
            onPickerCancelled?()
            return
        }

        var photos: [PhotoData] = []
        let group = DispatchGroup()

        for result in results {
            group.enter()

            let itemProvider = result.itemProvider

            if itemProvider.canLoadObject(ofClass: UIImage.self) {
                itemProvider.loadDataRepresentation(forTypeIdentifier: UTType.image.identifier) { data, error in
                    defer { group.leave() }

                    guard let imageData = data else { return }

                    let filename = itemProvider.suggestedName ?? "photo_\(result.assetIdentifier ?? UUID().uuidString)"

                    // Try to get the PHAsset for metadata
                    var creationDate: Date? = nil
                    var location: CLLocation? = nil

                    if let assetId = result.assetIdentifier {
                        let fetchResult = PHAsset.fetchAssets(withLocalIdentifiers: [assetId], options: nil)
                        if let asset = fetchResult.firstObject {
                            creationDate = asset.creationDate
                            location = asset.location
                        }
                    }

                    let photoData = PhotoData(
                        identifier: result.assetIdentifier ?? UUID().uuidString,
                        imageData: imageData,
                        filename: filename,
                        creationDate: creationDate,
                        location: location
                    )

                    DispatchQueue.main.async {
                        photos.append(photoData)
                    }
                }
            } else {
                group.leave()
            }
        }

        group.notify(queue: .main) {
            self.onPhotosSelected?(photos)
        }
    }
}

// MARK: - UIImagePickerControllerDelegate (iOS 13 fallback)
extension PhotoPickerManager: UIImagePickerControllerDelegate, UINavigationControllerDelegate {
    public func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey : Any]) {
        picker.dismiss(animated: true)

        guard let image = info[.originalImage] as? UIImage,
              let imageData = image.jpegData(compressionQuality: 0.9) else {
            onPickerCancelled?()
            return
        }

        var identifier = UUID().uuidString
        var creationDate: Date? = nil
        var location: CLLocation? = nil

        if let asset = info[.phAsset] as? PHAsset {
            identifier = asset.localIdentifier
            creationDate = asset.creationDate
            location = asset.location
        }

        let photoData = PhotoData(
            identifier: identifier,
            imageData: imageData,
            filename: "photo_\(identifier).jpg",
            creationDate: creationDate,
            location: location
        )

        onPhotosSelected?([photoData])
    }

    public func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
        picker.dismiss(animated: true)
        onPickerCancelled?()
    }
}
