#import "PhotoLibraryBridge.h"

#import <CoreLocation/CoreLocation.h>
#import <Network/Network.h>
#import <Photos/Photos.h>
#import <PhotosUI/PhotosUI.h>
#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>

static NSString * const kFootosNetworkAlertShownKey = @"footos.networkAlertShown";

@interface PhotoLibraryBridge () <WKScriptMessageHandler, CLLocationManagerDelegate>
@property (nonatomic, weak) WKWebView *webView;
@property (nonatomic, copy) NSString *dbPath;
@property (nonatomic, copy) NSString *thumbDir;
@property (nonatomic, assign) BOOL attached;
@property (nonatomic, assign) BOOL exporting;
@property (nonatomic, assign) BOOL networkAlertVisible;
@property (nonatomic, strong) id localNetworkBrowser;
@property (nonatomic, strong) NSMutableArray<NSString *> *pendingExportPaths;
@property (nonatomic, strong) NSLock *exportLock;
@property (nonatomic, strong) CLLocationManager *locationManager;
@property (nonatomic, assign) BOOL awaitingLocationForSync;
@property (nonatomic, copy) NSSet<NSString *> *knownLocalIds;
- (void)handOffPaths:(NSArray<NSString *> *)paths;
- (void)handOffPaths:(NSArray<NSString *> *)paths completion:(void (^)(void))completion;
- (void)handOffPaths:(NSArray<NSString *> *)paths attempt:(NSUInteger)attempt completion:(void (^)(void))completion;
- (NSString *)exportAsset:(PHAsset *)asset toDirectory:(NSString *)exportDir networkAccess:(BOOL)networkAccess;
- (void)enqueueExportedPath:(NSString *)path;
- (void)flushPendingExportBatchWithCompletion:(void (^)(void))completion;
- (void)showNetworkPermissionAlertForced:(BOOL)forced;
- (void)triggerLocalNetworkPermission;
- (void)checkWebViewLoadAndPrompt;
- (void)exportOriginalForIdentifier:(NSString *)identifier;
- (void)exportViewerStillForIdentifier:(NSString *)identifier;
- (void)requestLocationThenSync;
- (void)handleLocationAuthorization:(CLAuthorizationStatus)status;
- (void)handOffPreviews:(NSArray<NSDictionary *> *)items;
- (void)handOffPreviews:(NSArray<NSDictionary *> *)items completion:(void (^)(void))completion;
- (NSDictionary *)previewPayloadForAsset:(PHAsset *)asset
                              thumbPath:(NSString *)thumbPath
                               fileSize:(unsigned long long)fileSize;
- (void)writePreviewThumbnailForAsset:(PHAsset *)asset toPath:(NSString *)thumbPath fileSize:(unsigned long long *)fileSize;
- (NSString *)writeViewerStillForAsset:(PHAsset *)asset toDirectory:(NSString *)exportDir;
- (void)emitViewerStill:(NSString *)path identifier:(NSString *)identifier;
@end

@implementation PhotoLibraryBridge

+ (instancetype)shared {
    static PhotoLibraryBridge *instance;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        instance = [[PhotoLibraryBridge alloc] init];
    });
    return instance;
}

- (instancetype)init {
    self = [super init];
    if (self) {
        _pendingExportPaths = [NSMutableArray array];
        _exportLock = [[NSLock alloc] init];
        _locationManager = [[CLLocationManager alloc] init];
    }
    return self;
}

+ (void)load {
    dispatch_async(dispatch_get_main_queue(), ^{
        PhotoLibraryBridge *bridge = [PhotoLibraryBridge shared];
        [bridge triggerLocalNetworkPermission];
        [bridge startPollingForWebView];
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.8 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
            [bridge showNetworkPermissionAlertForced:NO];
        });
    });
}

- (void)startPollingForWebView {
    [self tryAttach];
    if (self.attached) {
        return;
    }
    __weak typeof(self) weakSelf = self;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.4 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
        [weakSelf startPollingForWebView];
    });
}

- (void)tryAttach {
    if (self.attached) {
        return;
    }
    WKWebView *webView = [self findWebView];
    if (!webView) {
        return;
    }
    self.webView = webView;
    [webView.configuration.userContentController addScriptMessageHandler:self name:@"footosPhotos"];
    self.attached = YES;
    [self eval:@"window.__FOOTOS_BRIDGE_READY__=true;window.dispatchEvent(new CustomEvent('footos-bridge-ready'));"];
    NSLog(@"[footos] PhotoKit bridge attached to WKWebView");
    __weak typeof(self) weakSelf = self;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(2.5 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
        [weakSelf checkWebViewLoadAndPrompt];
    });
}

- (void)triggerLocalNetworkPermission {
    if (self.localNetworkBrowser) {
        return;
    }
    nw_browse_descriptor_t descriptor = nw_browse_descriptor_create_bonjour_service("_http._tcp", "local.");
    nw_parameters_t parameters = nw_parameters_create();
    nw_browser_t browser = nw_browser_create(descriptor, parameters);
    nw_browser_set_queue(browser, dispatch_get_main_queue());
    nw_browser_set_browse_results_changed_handler(browser, ^(nw_browse_result_t result, nw_browse_result_t change, bool exact) {
        (void)result;
        (void)change;
        (void)exact;
    });
    nw_browser_start(browser);
    self.localNetworkBrowser = (id)browser;
    NSLog(@"[footos] Started local-network browse to request permission");
}

- (void)checkWebViewLoadAndPrompt {
    WKWebView *webView = self.webView;
    if (!webView) {
        [self showNetworkPermissionAlertForced:YES];
        return;
    }
    [webView evaluateJavaScript:@"(document.body && document.body.innerText) || ''"
              completionHandler:^(id result, NSError *error) {
        NSString *text = [result isKindOfClass:[NSString class]] ? result : @"";
        BOOL failed = error != nil
            || [text containsString:@"Failed to request"]
            || [text containsString:@"local network"]
            || [text containsString:@"did you grant"];
        if (failed) {
            [self showNetworkPermissionAlertForced:YES];
        }
    }];
}

- (void)showNetworkPermissionAlertForced:(BOOL)forced {
    if (self.networkAlertVisible) {
        return;
    }
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
    if (!forced && [defaults boolForKey:kFootosNetworkAlertShownKey]) {
        return;
    }

    UIViewController *vc = [self topViewController];
    if (!vc) {
        __weak typeof(self) weakSelf = self;
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.6 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
            [weakSelf showNetworkPermissionAlertForced:forced];
        });
        return;
    }

    [defaults setBool:YES forKey:kFootosNetworkAlertShownKey];
    self.networkAlertVisible = YES;

    UIAlertController *alert = [UIAlertController
        alertControllerWithTitle:@"需要打开网络权限"
        message:@"开发版安装后「无线数据」默认是关闭的，系统不会自动弹窗，页面会连不上电脑。\n\n请到：设置 → 足迹相册 → 无线数据 → 选「无线局域网」。\n\n如果弹出「本地网络」，请选允许。"
        preferredStyle:UIAlertControllerStyleAlert];

    [alert addAction:[UIAlertAction actionWithTitle:@"去设置" style:UIAlertActionStyleDefault handler:^(UIAlertAction *action) {
        self.networkAlertVisible = NO;
        NSURL *url = [NSURL URLWithString:UIApplicationOpenSettingsURLString];
        if (url) {
            [[UIApplication sharedApplication] openURL:url options:@{} completionHandler:nil];
        }
    }]];
    [alert addAction:[UIAlertAction actionWithTitle:@"已开启" style:UIAlertActionStyleCancel handler:^(UIAlertAction *action) {
        self.networkAlertVisible = NO;
    }]];
    [vc presentViewController:alert animated:YES completion:nil];
}

- (WKWebView *)findWebView {
    for (UIWindow *window in [self allWindows]) {
        WKWebView *found = [self findWebViewInView:window];
        if (found) {
            return found;
        }
    }
    return nil;
}

- (NSArray<UIWindow *> *)allWindows {
    NSMutableArray<UIWindow *> *windows = [NSMutableArray array];
    if (@available(iOS 13.0, *)) {
        for (UIScene *scene in UIApplication.sharedApplication.connectedScenes) {
            if (![scene isKindOfClass:[UIWindowScene class]]) {
                continue;
            }
            [windows addObjectsFromArray:((UIWindowScene *)scene).windows];
        }
    }
    if (windows.count == 0) {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        UIWindow *key = UIApplication.sharedApplication.keyWindow;
#pragma clang diagnostic pop
        if (key) {
            [windows addObject:key];
        }
    }
    return windows;
}

- (WKWebView *)findWebViewInView:(UIView *)view {
    if ([view isKindOfClass:[WKWebView class]]) {
        return (WKWebView *)view;
    }
    for (UIView *subview in view.subviews) {
        WKWebView *found = [self findWebViewInView:subview];
        if (found) {
            return found;
        }
    }
    return nil;
}

- (UIViewController *)topViewController {
    UIViewController *root = nil;
    for (UIWindow *window in [self allWindows]) {
        if (window.isKeyWindow || root == nil) {
            root = window.rootViewController;
        }
    }
    while (root.presentedViewController) {
        root = root.presentedViewController;
    }
    return root;
}

- (void)userContentController:(WKUserContentController *)userContentController
      didReceiveScriptMessage:(WKScriptMessage *)message {
    if (![message.body isKindOfClass:[NSDictionary class]]) {
        return;
    }
    NSDictionary *body = message.body;
    NSString *command = body[@"command"];
    if ([body[@"dbPath"] isKindOfClass:[NSString class]]) {
        self.dbPath = body[@"dbPath"];
    }
    if ([body[@"thumbDir"] isKindOfClass:[NSString class]]) {
        self.thumbDir = body[@"thumbDir"];
    }
    if ([body[@"knownIds"] isKindOfClass:[NSArray class]]) {
        self.knownLocalIds = [NSSet setWithArray:body[@"knownIds"]];
        NSLog(@"[footos] Received knownIds from JS: %lu db=%@",
              (unsigned long)self.knownLocalIds.count,
              self.dbPath ?: @"");
    }

    if ([command isEqualToString:@"checkStatus"]) {
        [self emitStatus:[self currentStatus]];
        return;
    }
    if ([command isEqualToString:@"requestAccessAndSync"]) {
        [self requestAccessAndSyncPresentLimitedPicker:NO];
        return;
    }
    if ([command isEqualToString:@"requestImport"]) {
        [self requestAccessAndSyncPresentLimitedPicker:YES];
        return;
    }
    if ([command isEqualToString:@"exportOriginal"]) {
        NSString *identifier = [body[@"identifier"] isKindOfClass:[NSString class]] ? body[@"identifier"] : @"";
        [self exportOriginalForIdentifier:identifier];
        return;
    }
    if ([command isEqualToString:@"exportViewerStill"]) {
        NSString *identifier = [body[@"identifier"] isKindOfClass:[NSString class]] ? body[@"identifier"] : @"";
        [self exportViewerStillForIdentifier:identifier];
        return;
    }
    if ([command isEqualToString:@"syncIfAuthorized"]) {
        PHAuthorizationStatus status = [self currentStatus];
        [self emitStatus:status];
        if (status == PHAuthorizationStatusAuthorized || status == PHAuthorizationStatusLimited) {
            [self requestLocationThenSync];
        }
        return;
    }
}

- (PHAuthorizationStatus)currentStatus {
    if (@available(iOS 14, *)) {
        return [PHPhotoLibrary authorizationStatusForAccessLevel:PHAccessLevelReadWrite];
    }
    return [PHPhotoLibrary authorizationStatus];
}

- (void)requestAccessAndSyncPresentLimitedPicker:(BOOL)presentLimitedPicker {
    PHAuthorizationStatus status = [self currentStatus];
    if (status == PHAuthorizationStatusNotDetermined) {
        __weak typeof(self) weakSelf = self;
        void (^handler)(PHAuthorizationStatus) = ^(PHAuthorizationStatus next) {
            dispatch_async(dispatch_get_main_queue(), ^{
                [weakSelf handleAuthorizationResult:next presentLimitedPicker:presentLimitedPicker];
            });
        };
        if (@available(iOS 14, *)) {
            [PHPhotoLibrary requestAuthorizationForAccessLevel:PHAccessLevelReadWrite handler:handler];
        } else {
            [PHPhotoLibrary requestAuthorization:handler];
        }
        return;
    }
    [self handleAuthorizationResult:status presentLimitedPicker:presentLimitedPicker];
}

- (void)handleAuthorizationResult:(PHAuthorizationStatus)status presentLimitedPicker:(BOOL)presentLimitedPicker {
    [self emitStatus:status];
    if (status == PHAuthorizationStatusAuthorized) {
        [self emitEvent:@"ios-permission-granted" detail:@{@"type": @"full"}];
        [self requestLocationThenSync];
        return;
    }
    if (status == PHAuthorizationStatusLimited) {
        [self emitEvent:@"ios-permission-granted" detail:@{@"type": @"limited"}];
        if (presentLimitedPicker) {
            UIViewController *vc = [self topViewController];
            if (vc) {
                [[PHPhotoLibrary sharedPhotoLibrary] presentLimitedLibraryPickerFromViewController:vc];
            }
        }
        [self requestLocationThenSync];
        return;
    }
    [self emitEvent:@"ios-permission-denied" detail:@{
        @"message": @"相册权限未授予。请在设置中允许访问所有照片。"
    }];
}

- (void)requestLocationThenSync {
    self.locationManager.delegate = self;
    CLAuthorizationStatus status;
    if (@available(iOS 14.0, *)) {
        status = self.locationManager.authorizationStatus;
    } else {
        status = [CLLocationManager authorizationStatus];
    }
    if (status == kCLAuthorizationStatusNotDetermined) {
        self.awaitingLocationForSync = YES;
        [self.locationManager requestWhenInUseAuthorization];
        return;
    }
    [self exportAuthorizedAssets];
}

- (void)locationManagerDidChangeAuthorization:(CLLocationManager *)manager {
    [self handleLocationAuthorization:manager.authorizationStatus];
}

- (void)locationManager:(CLLocationManager *)manager didChangeAuthorizationStatus:(CLAuthorizationStatus)status {
    [self handleLocationAuthorization:status];
}

- (void)handleLocationAuthorization:(CLAuthorizationStatus)status {
    if (!self.awaitingLocationForSync) {
        return;
    }
    if (status == kCLAuthorizationStatusNotDetermined) {
        return;
    }
    self.awaitingLocationForSync = NO;
    [self exportAuthorizedAssets];
}

- (void)exportAuthorizedAssets {
    if (self.exporting) {
        NSLog(@"[footos] Preview sync already running");
        return;
    }
    self.exporting = YES;

    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        PHFetchOptions *options = [[PHFetchOptions alloc] init];
        options.sortDescriptors = @[ [NSSortDescriptor sortDescriptorWithKey:@"creationDate" ascending:NO] ];
        PHFetchResult<PHAsset *> *images = [PHAsset fetchAssetsWithMediaType:PHAssetMediaTypeImage options:options];
        PHFetchResult<PHAsset *> *videos = [PHAsset fetchAssetsWithMediaType:PHAssetMediaTypeVideo options:options];

        NSMutableArray<PHAsset *> *assets = [NSMutableArray arrayWithCapacity:images.count + videos.count];
        [images enumerateObjectsUsingBlock:^(PHAsset *asset, NSUInteger idx, BOOL *stop) {
            [assets addObject:asset];
        }];
        [videos enumerateObjectsUsingBlock:^(PHAsset *asset, NSUInteger idx, BOOL *stop) {
            [assets addObject:asset];
        }];

        NSString *thumbDir = self.thumbDir.length > 0 ? self.thumbDir : NSTemporaryDirectory();
        [[NSFileManager defaultManager] createDirectoryAtPath:thumbDir
                                  withIntermediateDirectories:YES
                                                   attributes:nil
                                                        error:nil];

        NSSet<NSString *> *known = self.knownLocalIds ?: [NSSet set];
        NSMutableArray<PHAsset *> *pending = [NSMutableArray array];
        if (known.count == 0) {
            [pending addObjectsFromArray:assets];
            NSLog(@"[footos] DB has no ios-photo rows; re-upserting %lu assets (thumbs reused if present)",
                  (unsigned long)assets.count);
        } else {
            for (PHAsset *asset in assets) {
                if (![known containsObject:asset.localIdentifier]) {
                    [pending addObject:asset];
                }
            }
            NSLog(@"[footos] Incremental preview sync: %lu new / %lu total (jsKnown=%lu)",
                  (unsigned long)pending.count,
                  (unsigned long)assets.count,
                  (unsigned long)known.count);
        }

        if (pending.count == 0) {
            dispatch_async(dispatch_get_main_queue(), ^{
                self.exporting = NO;
                [self emitEvent:@"ios-sync-skipped" detail:@{
                    @"reason": @"up_to_date",
                    @"total": @(assets.count)
                }];
            });
            return;
        }

        dispatch_async(dispatch_get_main_queue(), ^{
            [self emitEvent:@"ios-import-started" detail:@{
                @"current": @0,
                @"total": @(pending.count)
            }];
        });

        dispatch_queue_t queue = dispatch_queue_create("app.footos.preview-sync", DISPATCH_QUEUE_CONCURRENT);
        dispatch_semaphore_t slots = dispatch_semaphore_create(8);
        dispatch_group_t group = dispatch_group_create();
        NSMutableArray<NSDictionary *> *batch = [NSMutableArray array];
        __block NSUInteger done = 0;
        NSUInteger total = pending.count;

        for (PHAsset *asset in pending) {
            dispatch_group_async(group, queue, ^{
                dispatch_semaphore_wait(slots, DISPATCH_TIME_FOREVER);
                NSString *safeId = [self safeIdForAsset:asset];
                NSString *thumbPath = [thumbDir stringByAppendingPathComponent:
                                       [NSString stringWithFormat:@"%@.jpg", safeId]];
                unsigned long long fileSize = 0;
                if (![[NSFileManager defaultManager] fileExistsAtPath:thumbPath]) {
                    [self writePreviewThumbnailForAsset:asset toPath:thumbPath fileSize:&fileSize];
                } else {
                    NSDictionary *attrs = [[NSFileManager defaultManager] attributesOfItemAtPath:thumbPath error:nil];
                    fileSize = [attrs[NSFileSize] unsignedLongLongValue];
                }
                NSDictionary *item = [self previewPayloadForAsset:asset thumbPath:thumbPath fileSize:fileSize];

                NSArray<NSDictionary *> *toSend = nil;
                NSUInteger current;
                [self.exportLock lock];
                [batch addObject:item];
                done += 1;
                current = done;
                if (batch.count >= 80) {
                    toSend = [batch copy];
                    [batch removeAllObjects];
                }
                [self.exportLock unlock];

                dispatch_async(dispatch_get_main_queue(), ^{
                    [self emitEvent:@"ios-export-progress" detail:@{
                        @"current": @(current),
                        @"total": @(total),
                        @"phase": @"Preview"
                    }];
                    if (toSend.count > 0) {
                        [self handOffPreviews:toSend];
                    }
                });
                dispatch_semaphore_signal(slots);
            });
        }
        dispatch_group_wait(group, DISPATCH_TIME_FOREVER);

        NSArray<NSDictionary *> *remaining = nil;
        [self.exportLock lock];
        if (batch.count > 0) {
            remaining = [batch copy];
            [batch removeAllObjects];
        }
        [self.exportLock unlock];

        dispatch_async(dispatch_get_main_queue(), ^{
            void (^finish)(void) = ^{
                self.exporting = NO;
                [self emitEvent:@"ios-export-complete" detail:@{@"exported": @(total)}];
            };
            if (remaining.count > 0) {
                [self handOffPreviews:remaining completion:finish];
            } else {
                finish();
            }
        });
    });
}

- (void)writePreviewThumbnailForAsset:(PHAsset *)asset toPath:(NSString *)thumbPath fileSize:(unsigned long long *)fileSize {
    PHImageRequestOptions *opts = [[PHImageRequestOptions alloc] init];
    opts.synchronous = YES;
    opts.networkAccessAllowed = NO;
    opts.resizeMode = PHImageRequestOptionsResizeModeFast;
    opts.deliveryMode = PHImageRequestOptionsDeliveryModeFastFormat;

    __block UIImage *image = nil;
    [[PHImageManager defaultManager] requestImageForAsset:asset
                                               targetSize:CGSizeMake(256, 256)
                                              contentMode:PHImageContentModeAspectFill
                                                  options:opts
                                            resultHandler:^(UIImage *result, NSDictionary *info) {
        (void)info;
        image = result;
    }];
    if (!image) {
        return;
    }
    NSData *data = UIImageJPEGRepresentation(image, 0.82);
    if (data.length == 0) {
        return;
    }
    [data writeToFile:thumbPath atomically:YES];
    if (fileSize) {
        *fileSize = data.length;
    }
}

- (NSString *)writeViewerStillForAsset:(PHAsset *)asset toDirectory:(NSString *)exportDir {
    NSString *stillDir = [exportDir stringByAppendingPathComponent:@"stills"];
    [[NSFileManager defaultManager] createDirectoryAtPath:stillDir
                              withIntermediateDirectories:YES
                                               attributes:nil
                                                    error:nil];
    NSString *outPath = [stillDir stringByAppendingPathComponent:
        [NSString stringWithFormat:@"%@.jpg", [self safeIdForAsset:asset]]];
    NSDictionary *attrs = [[NSFileManager defaultManager] attributesOfItemAtPath:outPath error:nil];
    if ([attrs[NSFileSize] unsignedLongLongValue] > 4096) {
        return outPath;
    }

    PHImageRequestOptions *opts = [[PHImageRequestOptions alloc] init];
    opts.synchronous = YES;
    opts.networkAccessAllowed = YES;
    opts.resizeMode = PHImageRequestOptionsResizeModeExact;
    opts.deliveryMode = PHImageRequestOptionsDeliveryModeHighQualityFormat;
    opts.version = PHImageRequestOptionsVersionCurrent;

    CGFloat maxSide = 2048;
    CGFloat width = (CGFloat)MAX(asset.pixelWidth, 1);
    CGFloat height = (CGFloat)MAX(asset.pixelHeight, 1);
    CGFloat longSide = MAX(width, height);
    if (longSide > maxSide) {
        CGFloat scale = maxSide / longSide;
        width *= scale;
        height *= scale;
    }

    __block UIImage *image = nil;
    [[PHImageManager defaultManager] requestImageForAsset:asset
                                               targetSize:CGSizeMake(width, height)
                                              contentMode:PHImageContentModeAspectFit
                                                  options:opts
                                            resultHandler:^(UIImage *result, NSDictionary *info) {
        if ([info[PHImageResultIsDegradedKey] boolValue] && image != nil) {
            return;
        }
        image = result;
    }];
    if (!image) {
        return nil;
    }
    NSData *data = UIImageJPEGRepresentation(image, 0.9);
    if (data.length == 0) {
        return nil;
    }
    [data writeToFile:outPath atomically:YES];
    return outPath;
}

- (void)emitViewerStill:(NSString *)path identifier:(NSString *)identifier {
    [self emitEvent:@"ios-viewer-still-ready" detail:@{
        @"identifier": identifier ?: @"",
        @"path": path ?: @""
    }];
    NSString *js = [NSString stringWithFormat:
        @"(function(){if(window.__handleIOSViewerStill)window.__handleIOSViewerStill(%@,%@);})()",
        [self jsString:identifier ?: @""],
        [self jsString:path ?: @""]
    ];
    [self eval:js];
}

- (NSDictionary *)previewPayloadForAsset:(PHAsset *)asset
                              thumbPath:(NSString *)thumbPath
                               fileSize:(unsigned long long)fileSize {
    NSMutableDictionary *item = [NSMutableDictionary dictionary];
    item[@"path"] = [NSString stringWithFormat:@"ios-photo://%@", asset.localIdentifier];
    if ([[NSFileManager defaultManager] fileExistsAtPath:thumbPath]) {
        item[@"thumb_path"] = thumbPath;
    }
    item[@"width"] = @(asset.pixelWidth);
    item[@"height"] = @(asset.pixelHeight);
    item[@"file_size"] = @(fileSize);
    if (asset.location) {
        item[@"lat"] = @(asset.location.coordinate.latitude);
        item[@"lon"] = @(asset.location.coordinate.longitude);
    }
    if (asset.creationDate) {
        NSDateFormatter *formatter = [[NSDateFormatter alloc] init];
        formatter.locale = [NSLocale localeWithLocaleIdentifier:@"en_US_POSIX"];
        formatter.dateFormat = @"yyyy-MM-dd'T'HH:mm:ss";
        item[@"date_taken"] = [formatter stringFromDate:asset.creationDate];
    }
    if (asset.mediaType == PHAssetMediaTypeVideo) {
        item[@"media_type"] = @"video";
        item[@"duration_ms"] = @((unsigned long long)(asset.duration * 1000.0));
    } else if (asset.mediaSubtypes & PHAssetMediaSubtypePhotoLive) {
        item[@"media_type"] = @"live_photo";
    } else {
        item[@"media_type"] = @"photo";
    }
    return item;
}

- (void)handOffPreviews:(NSArray<NSDictionary *> *)items {
    [self handOffPreviews:items completion:nil];
}

- (void)handOffPreviews:(NSArray<NSDictionary *> *)items completion:(void (^)(void))completion {
    NSData *data = [NSJSONSerialization dataWithJSONObject:items options:0 error:nil];
    NSString *json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] ?: @"[]";
    NSString *js = [NSString stringWithFormat:
        @"(function(){if(!window.__handleIOSPreviewBatch)return false;"
         "window.__handleIOSPreviewBatch(%@,%@,%@);return true;})()",
        json,
        [self jsString:self.dbPath ?: @""],
        [self jsString:self.thumbDir ?: @""]
    ];
    WKWebView *webView = self.webView;
    if (!webView) {
        if (completion) {
            completion();
        }
        return;
    }
    [webView evaluateJavaScript:js completionHandler:^(id result, NSError *error) {
        if (error) {
            NSLog(@"[footos] Preview handoff JS error: %@", error);
        } else if (![result isKindOfClass:[NSNumber class]] || ![result boolValue]) {
            NSLog(@"[footos] Preview handoff dropped (%lu items): JS handler missing",
                  (unsigned long)items.count);
        }
        if (completion) {
            completion();
        }
    }];
}

- (void)exportViewerStillForIdentifier:(NSString *)identifier {
    if (identifier.length == 0) {
        return;
    }
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        PHFetchResult<PHAsset *> *result = [PHAsset fetchAssetsWithLocalIdentifiers:@[identifier] options:nil];
        PHAsset *asset = result.firstObject;
        if (!asset) {
            dispatch_async(dispatch_get_main_queue(), ^{
                [self emitViewerStill:@"" identifier:identifier];
            });
            return;
        }
        NSString *photosDir = self.thumbDir.length > 0
            ? [[self.thumbDir stringByDeletingLastPathComponent] stringByAppendingPathComponent:@"Photos"]
            : [NSTemporaryDirectory() stringByAppendingPathComponent:@"footos-originals"];
        [[NSFileManager defaultManager] createDirectoryAtPath:photosDir
                                  withIntermediateDirectories:YES
                                                   attributes:nil
                                                        error:nil];
        NSString *stillPath = [self writeViewerStillForAsset:asset toDirectory:photosDir];
        dispatch_async(dispatch_get_main_queue(), ^{
            [self emitViewerStill:stillPath ?: @"" identifier:identifier];
        });
    });
}

- (void)exportOriginalForIdentifier:(NSString *)identifier {
    if (identifier.length == 0) {
        return;
    }
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        PHFetchResult<PHAsset *> *result = [PHAsset fetchAssetsWithLocalIdentifiers:@[identifier] options:nil];
        PHAsset *asset = result.firstObject;
        if (!asset) {
            dispatch_async(dispatch_get_main_queue(), ^{
                [self emitEvent:@"ios-original-ready" detail:@{@"identifier": identifier, @"error": @"not_found"}];
            });
            return;
        }
        NSString *photosDir = self.thumbDir.length > 0
            ? [[self.thumbDir stringByDeletingLastPathComponent] stringByAppendingPathComponent:@"Photos"]
            : [NSTemporaryDirectory() stringByAppendingPathComponent:@"footos-originals"];
        [[NSFileManager defaultManager] createDirectoryAtPath:photosDir
                                  withIntermediateDirectories:YES
                                                   attributes:nil
                                                        error:nil];
        if (asset.mediaType == PHAssetMediaTypeVideo ||
            (asset.mediaSubtypes & PHAssetMediaSubtypePhotoLive)) {
            NSString *stillPath = [self writeViewerStillForAsset:asset toDirectory:photosDir];
            if (stillPath.length > 0) {
                dispatch_async(dispatch_get_main_queue(), ^{
                    [self emitViewerStill:stillPath identifier:identifier];
                });
            }
        }
        NSString *path = [self exportAsset:asset toDirectory:photosDir networkAccess:YES];
        dispatch_async(dispatch_get_main_queue(), ^{
            NSMutableDictionary *detail = [@{@"identifier": identifier} mutableCopy];
            if (path.length > 0) {
                detail[@"path"] = path;
            } else {
                detail[@"error"] = @"export_failed";
            }
            [self emitEvent:@"ios-original-ready" detail:detail];
            NSString *js = [NSString stringWithFormat:
                @"(function(){if(window.__handleIOSOriginal)window.__handleIOSOriginal(%@,%@);})()",
                [self jsString:identifier],
                [self jsString:path ?: @""]
            ];
            [self eval:js];
        });
    });
}

- (NSArray<PHAsset *> *)exportAssets:(NSArray<PHAsset *> *)assets
                         toDirectory:(NSString *)exportDir
                       networkAccess:(BOOL)networkAccess
                               total:(NSUInteger)total
                               phase:(NSString *)phase {
    if (assets.count == 0) {
        return @[];
    }

    dispatch_queue_t queue = dispatch_queue_create("app.footos.photo-export", DISPATCH_QUEUE_CONCURRENT);
    dispatch_semaphore_t slots = dispatch_semaphore_create(6);
    dispatch_group_t group = dispatch_group_create();
    NSMutableArray<PHAsset *> *failed = [NSMutableArray array];
    NSLock *failLock = [[NSLock alloc] init];
    __block NSUInteger done = 0;

    for (PHAsset *asset in assets) {
        dispatch_group_async(group, queue, ^{
            dispatch_semaphore_wait(slots, DISPATCH_TIME_FOREVER);
            NSString *path = [self exportAsset:asset toDirectory:exportDir networkAccess:networkAccess];
            if (path.length > 0) {
                [self enqueueExportedPath:path];
            } else if (!networkAccess) {
                [failLock lock];
                [failed addObject:asset];
                [failLock unlock];
            }
            NSUInteger current;
            [self.exportLock lock];
            done += 1;
            current = done;
            [self.exportLock unlock];
            dispatch_async(dispatch_get_main_queue(), ^{
                [self emitEvent:@"ios-export-progress" detail:@{
                    @"current": @(current),
                    @"total": @(total),
                    @"phase": phase
                }];
            });
            dispatch_semaphore_signal(slots);
        });
    }
    dispatch_group_wait(group, DISPATCH_TIME_FOREVER);
    return [failed copy];
}

- (void)enqueueExportedPath:(NSString *)path {
    NSArray<NSString *> *toSend = nil;
    [self.exportLock lock];
    [self.pendingExportPaths addObject:path];
    if (self.pendingExportPaths.count >= 60) {
        toSend = [self.pendingExportPaths copy];
        [self.pendingExportPaths removeAllObjects];
    }
    [self.exportLock unlock];
    if (toSend.count > 0) {
        dispatch_async(dispatch_get_main_queue(), ^{
            [self handOffPaths:toSend];
        });
    }
}

- (void)flushPendingExportBatchWithCompletion:(void (^)(void))completion {
    NSArray<NSString *> *toSend = nil;
    [self.exportLock lock];
    if (self.pendingExportPaths.count > 0) {
        toSend = [self.pendingExportPaths copy];
        [self.pendingExportPaths removeAllObjects];
    }
    [self.exportLock unlock];
    if (toSend.count > 0) {
        [self handOffPaths:toSend completion:completion];
        return;
    }
    if (completion) {
        completion();
    }
}

- (NSString *)exportAsset:(PHAsset *)asset toDirectory:(NSString *)exportDir {
    return [self exportAsset:asset toDirectory:exportDir networkAccess:YES];
}

- (NSString *)exportAsset:(PHAsset *)asset toDirectory:(NSString *)exportDir networkAccess:(BOOL)networkAccess {
    NSArray<PHAssetResource *> *resources = [PHAssetResource assetResourcesForAsset:asset];
    PHAssetResource *resource = nil;
    for (PHAssetResource *candidate in resources) {
        if (candidate.type == PHAssetResourceTypePhoto ||
            candidate.type == PHAssetResourceTypeVideo ||
            candidate.type == PHAssetResourceTypeFullSizePhoto ||
            candidate.type == PHAssetResourceTypeFullSizeVideo) {
            resource = candidate;
            if (candidate.type == PHAssetResourceTypePhoto || candidate.type == PHAssetResourceTypeVideo) {
                break;
            }
        }
    }
    if (!resource) {
        resource = resources.firstObject;
    }
    if (!resource) {
        return nil;
    }

    NSString *safeId = [self safeIdForAsset:asset];
    NSString *original = resource.originalFilename.length > 0 ? resource.originalFilename : @"photo.jpg";
    NSString *filename = [NSString stringWithFormat:@"%@-%@", safeId, original];
    NSString *outPath = [exportDir stringByAppendingPathComponent:filename];
    if ([[NSFileManager defaultManager] fileExistsAtPath:outPath]) {
        NSDictionary *attrs = [[NSFileManager defaultManager] attributesOfItemAtPath:outPath error:nil];
        if ([attrs[NSFileSize] unsignedLongLongValue] > 0) {
            return outPath;
        }
        [[NSFileManager defaultManager] removeItemAtPath:outPath error:nil];
    }

    dispatch_semaphore_t sema = dispatch_semaphore_create(0);
    __block BOOL ok = NO;
    PHAssetResourceRequestOptions *options = [[PHAssetResourceRequestOptions alloc] init];
    options.networkAccessAllowed = networkAccess;
    [[PHAssetResourceManager defaultManager] writeDataForAssetResource:resource
                                                                toFile:[NSURL fileURLWithPath:outPath]
                                                               options:options
                                                     completionHandler:^(NSError * _Nullable error) {
        ok = (error == nil);
        if (error) {
            NSLog(@"[footos] Export failed for %@: %@", asset.localIdentifier, error);
        }
        dispatch_semaphore_signal(sema);
    }];
    int64_t timeoutSec = networkAccess ? 60 : 8;
    long timedOut = dispatch_semaphore_wait(sema, dispatch_time(DISPATCH_TIME_NOW, timeoutSec * NSEC_PER_SEC));
    if (timedOut != 0 || !ok) {
        [[NSFileManager defaultManager] removeItemAtPath:outPath error:nil];
        return nil;
    }
    return outPath;
}

- (void)handOffPaths:(NSArray<NSString *> *)paths {
    [self handOffPaths:paths attempt:0 completion:nil];
}

- (void)handOffPaths:(NSArray<NSString *> *)paths completion:(void (^)(void))completion {
    [self handOffPaths:paths attempt:0 completion:completion];
}

- (void)handOffPaths:(NSArray<NSString *> *)paths attempt:(NSUInteger)attempt completion:(void (^)(void))completion {
    NSData *data = [NSJSONSerialization dataWithJSONObject:paths options:0 error:nil];
    NSString *json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] ?: @"[]";
    NSString *js = [NSString stringWithFormat:
        @"(function(){if(!window.__handleIOSImportPaths)return false;"
         "window.__handleIOSImportPaths(%@,%@,%@);return true;})()",
        json,
        [self jsString:self.dbPath ?: @""],
        [self jsString:self.thumbDir ?: @""]
    ];
    WKWebView *webView = self.webView;
    if (!webView) {
        if (completion) {
            completion();
        }
        return;
    }
    __weak typeof(self) weakSelf = self;
    [webView evaluateJavaScript:js completionHandler:^(id result, NSError *error) {
        BOOL accepted = [result isKindOfClass:[NSNumber class]] && [result boolValue];
        if (accepted) {
            NSLog(@"[footos] Handed off %lu photo paths to JS", (unsigned long)paths.count);
            if (completion) {
                completion();
            }
            return;
        }
        if (attempt >= 20) {
            NSLog(@"[footos] JS import handler never became ready");
            if (completion) {
                completion();
            }
            return;
        }
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.5 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
            [weakSelf handOffPaths:paths attempt:attempt + 1 completion:completion];
        });
    }];
}

- (NSString *)safeIdForAsset:(PHAsset *)asset {
    return [[asset.localIdentifier stringByReplacingOccurrencesOfString:@"/" withString:@"_"]
            stringByReplacingOccurrencesOfString:@":" withString:@"_"];
}

- (NSSet<NSString *> *)importedSafeIds {
    if (self.thumbDir.length == 0) {
        return [NSSet set];
    }
    NSString *photosDir = [[self.thumbDir stringByDeletingLastPathComponent]
                           stringByAppendingPathComponent:@"Photos"];
    NSArray<NSString *> *files = [[NSFileManager defaultManager] contentsOfDirectoryAtPath:photosDir error:nil];
    if (files.count == 0) {
        return [NSSet set];
    }
    NSMutableSet<NSString *> *ids = [NSMutableSet setWithCapacity:files.count];
    for (NSString *file in files) {
        NSRange dash = [file rangeOfString:@"-"];
        if (dash.location != NSNotFound && dash.location > 0) {
            [ids addObject:[file substringToIndex:dash.location]];
        }
    }
    return ids;
}

- (void)emitStatus:(PHAuthorizationStatus)status {
    [self emitEvent:@"ios-permission-status" detail:@{@"status": @(status)}];
}

- (void)emitEvent:(NSString *)name detail:(NSDictionary *)detail {
    NSData *data = [NSJSONSerialization dataWithJSONObject:detail ?: @{} options:0 error:nil];
    NSString *json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] ?: @"{}";
    NSString *js = [NSString stringWithFormat:
        @"window.dispatchEvent(new CustomEvent('%@',{detail:%@}));",
        name,
        json
    ];
    [self eval:js];
}

- (NSString *)jsString:(NSString *)value {
    NSData *data = [NSJSONSerialization dataWithJSONObject:@[value ?: @""] options:0 error:nil];
    NSString *array = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    if (array.length >= 2) {
        return [array substringWithRange:NSMakeRange(1, array.length - 2)];
    }
    return @"\"\"";
}

- (void)eval:(NSString *)js {
    WKWebView *webView = self.webView;
    if (!webView) {
        return;
    }
    [webView evaluateJavaScript:js completionHandler:nil];
}

@end
