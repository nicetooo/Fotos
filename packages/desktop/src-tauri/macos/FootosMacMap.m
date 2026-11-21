// The system map (MapKit) for the macOS app.
//
// Same arrangement as on iOS (gen/apple/Sources/footos/FootosMapView.m): an
// MKMapView sits underneath the web view, which leaves the map's rectangle
// unpainted. The page reports where that rectangle is and which parts of it
// its own controls cover; mouse events anywhere else in it go to the map.
// The map reports its visible region; the page asks Rust what is in it and
// hands back the clusters to draw.
//
// Entry point: footos_mac_map_command(), called on the main thread.

#import <AppKit/AppKit.h>
#import <ImageIO/ImageIO.h>
#import <MapKit/MapKit.h>
#import <QuartzCore/QuartzCore.h>
#import <WebKit/WebKit.h>
#import <dlfcn.h>
#import <objc/message.h>
#import <objc/runtime.h>

// MapLibre's zoom convention, which the Rust clustering is written for: the
// world is 512 px wide at zoom 0.
static const double kWorldPx = 512.0;
static const CGFloat kSingleSize = 46.0;
static const CGFloat kClusterSize = 58.0;
static const NSTimeInterval kRegionReportInterval = 0.12;

#pragma mark - WGS-84 <-> GCJ-02

// Inside mainland China the system map is drawn in GCJ-02, while photo GPS is
// WGS-84; unconverted points land a few hundred metres off.

static BOOL FMOutsideChina(double lat, double lon) {
    return lon < 72.004 || lon > 137.8347 || lat < 0.8293 || lat > 55.8271;
}

static double FMTransformLat(double x, double y) {
    double ret = -100.0 + 2.0 * x + 3.0 * y + 0.2 * y * y + 0.1 * x * y + 0.2 * sqrt(fabs(x));
    ret += (20.0 * sin(6.0 * x * M_PI) + 20.0 * sin(2.0 * x * M_PI)) * 2.0 / 3.0;
    ret += (20.0 * sin(y * M_PI) + 40.0 * sin(y / 3.0 * M_PI)) * 2.0 / 3.0;
    ret += (160.0 * sin(y / 12.0 * M_PI) + 320.0 * sin(y * M_PI / 30.0)) * 2.0 / 3.0;
    return ret;
}

static double FMTransformLon(double x, double y) {
    double ret = 300.0 + x + 2.0 * y + 0.1 * x * x + 0.1 * x * y + 0.1 * sqrt(fabs(x));
    ret += (20.0 * sin(6.0 * x * M_PI) + 20.0 * sin(2.0 * x * M_PI)) * 2.0 / 3.0;
    ret += (20.0 * sin(x * M_PI) + 40.0 * sin(x / 3.0 * M_PI)) * 2.0 / 3.0;
    ret += (150.0 * sin(x / 12.0 * M_PI) + 300.0 * sin(x / 30.0 * M_PI)) * 2.0 / 3.0;
    return ret;
}

static CLLocationCoordinate2D FMGCJOffset(double lat, double lon) {
    const double a = 6378245.0;
    const double ee = 0.00669342162296594323;
    double dLat = FMTransformLat(lon - 105.0, lat - 35.0);
    double dLon = FMTransformLon(lon - 105.0, lat - 35.0);
    double radLat = lat / 180.0 * M_PI;
    double magic = sin(radLat);
    magic = 1 - ee * magic * magic;
    double sqrtMagic = sqrt(magic);
    dLat = (dLat * 180.0) / ((a * (1 - ee)) / (magic * sqrtMagic) * M_PI);
    dLon = (dLon * 180.0) / (a / sqrtMagic * cos(radLat) * M_PI);
    return CLLocationCoordinate2DMake(dLat, dLon);
}

#pragma mark - Annotation

@interface FootosAnnotation : NSObject <MKAnnotation>
@property (nonatomic, assign) CLLocationCoordinate2D coordinate;
@property (nonatomic, copy) NSString *key;
@property (nonatomic, assign) NSInteger count;
@property (nonatomic, copy) NSString *label;
@property (nonatomic, copy) NSString *thumb;
@end

@implementation FootosAnnotation
@end

@class FootosMacMap;

@interface FootosAnnotationView : MKAnnotationView
@property (nonatomic, strong) NSView *picture;
@property (nonatomic, strong) NSView *pill;
@property (nonatomic, strong) NSTextField *countLabel;
@property (nonatomic, copy) NSString *shownThumb;
@property (nonatomic, strong) NSTrackingArea *tracking;
- (void)configure;
@end

static NSCache<NSString *, NSImage *> *FMThumbCache(void) {
    static NSCache *cache;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        cache = [[NSCache alloc] init];
        cache.countLimit = 400;
    });
    return cache;
}

static NSImage *FMLoadThumb(NSString *path) {
    if (path.length == 0) {
        return nil;
    }
    CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path], NULL);
    if (!source) {
        return nil;
    }
    NSDictionary *options = @{
        (id)kCGImageSourceCreateThumbnailFromImageAlways: @YES,
        (id)kCGImageSourceCreateThumbnailWithTransform: @YES,
        (id)kCGImageSourceShouldCacheImmediately: @YES,
        (id)kCGImageSourceThumbnailMaxPixelSize: @(kClusterSize * 3),
    };
    CGImageRef cg = CGImageSourceCreateThumbnailAtIndex(source, 0, (__bridge CFDictionaryRef)options);
    CFRelease(source);
    if (!cg) {
        return nil;
    }
    NSImage *image = [[NSImage alloc] initWithCGImage:cg size:NSZeroSize];
    CGImageRelease(cg);
    return image;
}

#pragma mark - Map

@interface FootosMacMap : NSObject <MKMapViewDelegate>
@property (nonatomic, strong, nullable) MKMapView *mapView;
@property (nonatomic, weak, nullable) WKWebView *webView;
@property (nonatomic, strong) NSMutableDictionary<NSString *, FootosAnnotation *> *annotations;
/// Where the web UI lies over the map, in web view coordinates (origin top left).
@property (nonatomic, copy) NSArray<NSValue *> *overlayRects;
@property (nonatomic, assign) BOOL overlaysEnabled;
@property (nonatomic, assign) CGRect mapRectInWebView;
/// Height of the page's viewport as the page reports it.
@property (nonatomic, assign) CGFloat pageHeight;
@property (nonatomic, assign) BOOL useGCJ;
@property (nonatomic, assign) CFTimeInterval lastRegionReport;
@property (nonatomic, assign) BOOL regionReportPending;
+ (instancetype)shared;
- (BOOL)passesThroughPoint:(NSPoint)pointInPage;
- (NSPoint)pagePointFromView:(NSPoint)point;
- (void)hoverAnnotationView:(FootosAnnotationView *)view inside:(BOOL)inside;
@end

@implementation FootosAnnotationView

- (instancetype)initWithAnnotation:(id<MKAnnotation>)annotation reuseIdentifier:(NSString *)reuseIdentifier {
    self = [super initWithAnnotation:annotation reuseIdentifier:reuseIdentifier];
    if (self) {
        self.wantsLayer = YES;
        self.canShowCallout = NO;
        self.collisionMode = MKAnnotationViewCollisionModeNone;
        self.displayPriority = MKFeatureDisplayPriorityRequired;

        _picture = [[NSView alloc] initWithFrame:NSZeroRect];
        _picture.wantsLayer = YES;
        _picture.layer.backgroundColor = [NSColor colorWithWhite:0.75 alpha:1].CGColor;
        _picture.layer.borderColor = NSColor.whiteColor.CGColor;
        _picture.layer.borderWidth = 2;
        _picture.layer.masksToBounds = YES;
        _picture.layer.contentsGravity = kCAGravityResizeAspectFill;
        [self addSubview:_picture];

        _pill = [[NSView alloc] initWithFrame:NSZeroRect];
        _pill.wantsLayer = YES;
        _pill.layer.backgroundColor = [NSColor colorWithWhite:0 alpha:0.6].CGColor;
        _pill.layer.cornerRadius = 9;
        [self addSubview:_pill];

        _countLabel = [NSTextField labelWithString:@""];
        _countLabel.font = [NSFont systemFontOfSize:12 weight:NSFontWeightBold];
        _countLabel.textColor = NSColor.whiteColor;
        _countLabel.alignment = NSTextAlignmentCenter;
        [_pill addSubview:_countLabel];
    }
    return self;
}

- (void)configure {
    FootosAnnotation *item = (FootosAnnotation *)self.annotation;
    if (![item isKindOfClass:[FootosAnnotation class]]) {
        return;
    }
    BOOL cluster = item.count > 1;
    CGFloat size = cluster ? kClusterSize : kSingleSize;
    self.frame = NSMakeRect(self.frame.origin.x, self.frame.origin.y, size, size);
    self.picture.frame = self.bounds;
    self.picture.layer.cornerRadius = size / 2;
    // Bigger clusters sit above smaller ones and above single photos.
    self.zPriority = (MKAnnotationViewZPriority)MIN(999, 500 + item.count / 10);

    self.pill.hidden = !cluster;
    if (cluster) {
        self.countLabel.stringValue = item.label.length ? item.label : [NSString stringWithFormat:@"%ld", (long)item.count];
        [self.countLabel sizeToFit];
        CGFloat w = MAX(26, self.countLabel.frame.size.width + 10);
        self.pill.frame = NSMakeRect((size - w) / 2, (size - 18) / 2, w, 18);
        CGFloat h = self.countLabel.frame.size.height;
        self.countLabel.frame = NSMakeRect(0, (18 - h) / 2, w, h);
    }

    NSString *thumb = item.thumb ?: @"";
    if ([thumb isEqualToString:self.shownThumb ?: @""] && self.picture.layer.contents) {
        return;
    }
    self.shownThumb = thumb;
    NSImage *cached = thumb.length ? [FMThumbCache() objectForKey:thumb] : nil;
    self.picture.layer.contents = cached;
    if (cached || thumb.length == 0) {
        return;
    }
    __weak typeof(self) weakSelf = self;
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        NSImage *image = FMLoadThumb(thumb);
        if (!image) {
            return;
        }
        [FMThumbCache() setObject:image forKey:thumb];
        dispatch_async(dispatch_get_main_queue(), ^{
            if ([weakSelf.shownThumb isEqualToString:thumb]) {
                weakSelf.picture.layer.contents = image;
            }
        });
    });
}

- (void)prepareForReuse {
    [super prepareForReuse];
    self.shownThumb = nil;
    self.picture.layer.contents = nil;
}

- (void)updateTrackingAreas {
    [super updateTrackingAreas];
    if (self.tracking) {
        [self removeTrackingArea:self.tracking];
    }
    self.tracking = [[NSTrackingArea alloc] initWithRect:self.bounds
                                                 options:NSTrackingMouseEnteredAndExited | NSTrackingActiveInKeyWindow
                                                   owner:self
                                                userInfo:nil];
    [self addTrackingArea:self.tracking];
}

- (void)mouseEntered:(NSEvent *)event {
    [[FootosMacMap shared] hoverAnnotationView:self inside:YES];
}

- (void)mouseExited:(NSEvent *)event {
    [[FootosMacMap shared] hoverAnnotationView:self inside:NO];
}

@end

// The web view's hit test, replaced on its class (see installHitTestOn:): a
// click on the map area that no web control covers is not the web view's, so
// the hit test falls through to the map view underneath. `point` is in the
// superview's coordinates.
static IMP gOriginalHitTest = NULL;
static NSView *FMWebViewHitTest(NSView *self, SEL _cmd, NSPoint point) {
    FootosMacMap *map = [FootosMacMap shared];
    if (self == map.webView) {
        NSPoint local = [self convertPoint:point fromView:self.superview];
        if ([map passesThroughPoint:[map pagePointFromView:local]]) {
            return nil;
        }
    }
    return ((NSView *(*)(id, SEL, NSPoint))gOriginalHitTest)(self, _cmd, point);
}

@implementation FootosMacMap

+ (instancetype)shared {
    static FootosMacMap *instance;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        instance = [[FootosMacMap alloc] init];
    });
    return instance;
}

- (instancetype)init {
    self = [super init];
    if (self) {
        _annotations = [NSMutableDictionary dictionary];
        _overlayRects = @[];
        _overlaysEnabled = NO;
        // See the iOS implementation: the time zone is the closest public
        // stand-in for "this machine is served the shifted (GCJ-02) map".
        NSString *zone = [NSTimeZone localTimeZone].name ?: @"";
        NSSet<NSString *> *mainland = [NSSet setWithObjects:
            @"Asia/Shanghai", @"Asia/Urumqi", @"Asia/Chongqing", @"Asia/Harbin", @"Asia/Kashgar", nil];
        _useGCJ = [mainland containsObject:zone];
        NSLog(@"[footos] map: time zone %@, gcj=%d", zone, _useGCJ);
    }
    return self;
}

#pragma mark Page coordinates

// The page measures from the top left of its viewport. Two things stand
// between that and the web view's own coordinates: the viewport can start
// below the top of the view (the web view reaches under the title bar, the
// page does not), and an NSView measures from the bottom left unless it is
// flipped.

/// How far below the top of the web view the page starts.
- (CGFloat)pageTopInset {
    WKWebView *webView = self.webView;
    if (!webView || self.pageHeight <= 0) {
        return 0;
    }
    CGFloat inset = webView.bounds.size.height - self.pageHeight;
    return (inset > 0 && inset < 200) ? inset : 0;
}

- (NSRect)viewRectFromPage:(CGRect)rect {
    WKWebView *webView = self.webView;
    CGFloat y = rect.origin.y + [self pageTopInset];
    if (!webView || webView.isFlipped) {
        return NSMakeRect(rect.origin.x, y, rect.size.width, rect.size.height);
    }
    return NSMakeRect(rect.origin.x, webView.bounds.size.height - y - rect.size.height,
                      rect.size.width, rect.size.height);
}

- (NSPoint)pagePointFromView:(NSPoint)point {
    WKWebView *webView = self.webView;
    CGFloat y = (!webView || webView.isFlipped) ? point.y : webView.bounds.size.height - point.y;
    return NSMakePoint(point.x, y - [self pageTopInset]);
}

#pragma mark Coordinates

- (CLLocationCoordinate2D)displayCoordinateForLat:(double)lat lon:(double)lon {
    if (!self.useGCJ || FMOutsideChina(lat, lon)) {
        return CLLocationCoordinate2DMake(lat, lon);
    }
    CLLocationCoordinate2D d = FMGCJOffset(lat, lon);
    return CLLocationCoordinate2DMake(lat + d.latitude, lon + d.longitude);
}

/// The WGS-84 coordinate a point shown on the map stands for.
- (CLLocationCoordinate2D)realCoordinate:(CLLocationCoordinate2D)shown {
    if (!self.useGCJ || FMOutsideChina(shown.latitude, shown.longitude)) {
        return shown;
    }
    CLLocationCoordinate2D d = FMGCJOffset(shown.latitude, shown.longitude);
    return CLLocationCoordinate2DMake(shown.latitude - d.latitude, shown.longitude - d.longitude);
}

- (double)zoomForLongitudeDelta:(double)lonDelta {
    CGFloat width = MAX(1, self.mapView.bounds.size.width);
    if (lonDelta <= 0) {
        return 22;
    }
    return log2(360.0 * width / (kWorldPx * lonDelta));
}

- (double)currentZoom {
    MKMapRect rect = self.mapView.visibleMapRect;
    return [self zoomForLongitudeDelta:rect.size.width / MKMapSizeWorld.width * 360.0];
}

/// The map rect showing `center` at `zoom`, for the current view size.
- (MKMapRect)mapRectForCenter:(CLLocationCoordinate2D)center zoom:(double)zoom {
    CGSize size = self.mapView.bounds.size;
    double lonDelta = 360.0 * MAX(1, size.width) / (kWorldPx * pow(2, zoom));
    double w = MKMapSizeWorld.width * lonDelta / 360.0;
    double h = w * MAX(1, size.height) / MAX(1, size.width);
    MKMapPoint c = MKMapPointForCoordinate(center);
    return MKMapRectMake(c.x - w / 2, c.y - h / 2, w, h);
}

#pragma mark Commands

- (void)handleCommand:(NSString *)command body:(NSDictionary *)body webView:(WKWebView *)webView {
    if ([command isEqualToString:@"mapAttach"]) {
        [self attachToWebView:webView frame:[self rectFrom:body] dark:[body[@"dark"] boolValue]];
    } else if ([command isEqualToString:@"mapDetach"]) {
        [self detach];
    } else if (!self.mapView) {
        return;
    } else if ([command isEqualToString:@"mapSetFrame"]) {
        [self setFrameInWebView:[self rectFrom:body]];
    } else if ([command isEqualToString:@"mapSetFeatures"]) {
        NSArray *features = [body[@"features"] isKindOfClass:[NSArray class]] ? body[@"features"] : @[];
        [self setFeatures:features];
    } else if ([command isEqualToString:@"mapFitBounds"]) {
        [self fitBounds:body];
    } else if ([command isEqualToString:@"mapEaseTo"]) {
        [self easeTo:body];
    } else if ([command isEqualToString:@"mapSetTheme"]) {
        [self applyDark:[body[@"dark"] boolValue]];
    } else if ([command isEqualToString:@"mapSetOverlays"]) {
        [self setOverlays:body];
    } else if ([command isEqualToString:@"mapDebugProbeViews"]) {
        [self debugProbeViews];
    } else if ([command isEqualToString:@"mapDebugHit"]) {
        // Who gets a click at this point of the page: the page or the map?
        NSView *content = self.webView.window.contentView;
        NSRect inView = [self viewRectFromPage:CGRectMake([body[@"x"] doubleValue], [body[@"y"] doubleValue], 0, 0)];
        NSPoint inWindow = [self.webView convertPoint:inView.origin toView:nil];
        NSView *hit = [content hitTest:[content.superview convertPoint:inWindow fromView:nil]];
        BOOL page = hit && self.webView && [hit isDescendantOf:self.webView];
        NSLog(@"[footos] hit %@ -> %@", body[@"label"] ?: @"?", page ? @"page" : @"map");
    } else if ([command isEqualToString:@"mapDebugMouseMove"]) {
        [self debugMouseMoveToPageX:[body[@"x"] doubleValue] y:[body[@"y"] doubleValue]];
    } else if ([command isEqualToString:@"mapDebugFront"]) {
        // The self-test measures frames, and a window behind another gets
        // none: keep it above the others for the length of the run.
        NSWindow *window = self.webView.window;
        window.level = NSFloatingWindowLevel;
        [window orderFrontRegardless];
    } else if ([command isEqualToString:@"mapDebugLog"]) {
        // A page error, into the app's log where the self-test can read it.
        NSLog(@"[footos] page: %@", body[@"text"]);
    } else if ([command isEqualToString:@"mapDebugSnapshot"]) {
        [self snapshotWindowTo:[body[@"path"] isKindOfClass:[NSString class]] ? body[@"path"] : nil];
    }
}

/// Asks each view involved for what AppKit asks it when the pointer moves over
/// the window (`-[NSView _interactiveBounds]`, from the tracking-area
/// manager). A view AppKit cannot answer for aborts the process there, which
/// is how a pointer over the window once crashed the app. Logging before each
/// call shows which view it was.
- (void)debugProbeViews {
    SEL probe = NSSelectorFromString(@"_interactiveBounds");
    // Every view in the window, not only ours: the map view has many of its own.
    NSMutableArray<NSView *> *views = [NSMutableArray array];
    NSView *root = self.webView.window.contentView.superview ?: self.webView.window.contentView;
    NSMutableArray<NSView *> *queue = root ? [NSMutableArray arrayWithObject:root] : [NSMutableArray array];
    while (queue.count) {
        NSView *view = queue.firstObject;
        [queue removeObjectAtIndex:0];
        [views addObject:view];
        [queue addObjectsFromArray:view.subviews];
    }
    for (NSView *view in views) {
        // Views that own tracking areas, and everything inside the web view:
        // WebKit puts its tracking areas on inner views once the window is
        // active, and those ask the web view above them.
        BOOL insideWeb = self.webView && [view isDescendantOf:self.webView];
        if (![view respondsToSelector:probe] || (view.trackingAreas.count == 0 && !insideWeb)) {
            continue;
        }
        NSLog(@"[footos] probe: %@ (tracking areas: %lu)", NSStringFromClass(object_getClass(view)),
              (unsigned long)view.trackingAreas.count);
        ((NSRect (*)(id, SEL))objc_msgSend)(view, probe);
    }
    NSLog(@"[footos] probe: all %lu views answered", (unsigned long)views.count);

    // Where a click would land: the middle of the map must reach the map
    // view, a corner of the page outside the map area must stay with the page.
    NSView *content = self.webView.window.contentView;
    NSView *(^hitAtPage)(CGFloat, CGFloat) = ^NSView *(CGFloat x, CGFloat y) {
        NSRect inView = [self viewRectFromPage:CGRectMake(x, y, 0, 0)];
        NSPoint inWindow = [self.webView convertPoint:inView.origin toView:nil];
        return [content hitTest:[content.superview convertPoint:inWindow fromView:nil]];
    };
    CGRect area = self.mapRectInWebView;
    NSView *onMap = hitAtPage(CGRectGetMidX(area), CGRectGetMidY(area));
    NSView *onPage = hitAtPage(CGRectGetMidX(area), CGRectGetMaxY(area) + 40);
    BOOL mapOK = onMap && self.mapView && [onMap isDescendantOf:self.mapView];
    BOOL pageOK = onPage && self.webView && [onPage isDescendantOf:self.webView];
    NSLog(@"[footos] probe: click on map -> %@ (%@), click below map -> %@ (%@)",
          NSStringFromClass([onMap class]), mapOK ? @"map, ok" : @"NOT the map",
          NSStringFromClass([onPage class]), pageOK ? @"page, ok" : @"NOT the page");
}

/// Sends the window a mouse-moved event at a point of the page, as if the
/// pointer were there. The self-test has no pointer of its own, and code that
/// only runs when one moves over the window (tracking areas, hover) would
/// otherwise go untested.
- (void)debugMouseMoveToPageX:(CGFloat)x y:(CGFloat)y {
    WKWebView *webView = self.webView;
    NSWindow *window = webView.window;
    if (!window) {
        return;
    }
    // Tracking areas are only live in the key window of the active app.
    if (!window.isKeyWindow) {
        [NSApp activateIgnoringOtherApps:YES];
        [window makeKeyAndOrderFront:nil];
    }
    NSRect inView = [self viewRectFromPage:CGRectMake(x, y, 0, 0)];
    NSPoint inWindow = [webView convertPoint:inView.origin toView:nil];
    NSEvent *event = [NSEvent mouseEventWithType:NSEventTypeMouseMoved
                                        location:inWindow
                                   modifierFlags:0
                                       timestamp:NSProcessInfo.processInfo.systemUptime
                                    windowNumber:window.windowNumber
                                         context:nil
                                     eventNumber:0
                                      clickCount:0
                                        pressure:0];
    [NSApp sendEvent:event];
}

/// A picture of the app's own window, for the self-test to look at: the map
/// and the page are composited by the window server, so only a window
/// capture shows what the person sees. Capturing one's own window needs no
/// screen-recording permission.
- (void)snapshotWindowTo:(NSString *)path {
    NSWindow *window = self.webView.window;
    if (!window || path.length == 0) {
        return;
    }
    typedef CGImageRef (*CreateImage)(CGRect, uint32_t, uint32_t, uint32_t);
    CreateImage create = (CreateImage)dlsym(RTLD_DEFAULT, "CGWindowListCreateImage");
    if (!create) {
        NSLog(@"[footos] map: window capture is not available on this system");
        return;
    }
    // kCGWindowListOptionIncludingWindow = 8, kCGWindowImageBoundsIgnoreFraming = 1.
    CGImageRef image = create(CGRectNull, 8, (uint32_t)window.windowNumber, 1);
    if (!image) {
        NSLog(@"[footos] map: window capture returned nothing");
        return;
    }
    NSBitmapImageRep *rep = [[NSBitmapImageRep alloc] initWithCGImage:image];
    CGImageRelease(image);
    NSData *png = [rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
    BOOL ok = [png writeToFile:path atomically:YES];
    NSLog(@"[footos] map: window snapshot %@ -> %@ (map frame %@, web bounds %@)", ok ? @"written" : @"FAILED",
          path, NSStringFromRect(self.mapView.frame), NSStringFromRect(self.webView.bounds));
}

- (CGRect)rectFrom:(NSDictionary *)body {
    if ([body[@"vh"] doubleValue] > 0) {
        self.pageHeight = [body[@"vh"] doubleValue];
    }
    return CGRectMake([body[@"x"] doubleValue], [body[@"y"] doubleValue],
                      [body[@"w"] doubleValue], [body[@"h"] doubleValue]);
}

- (void)applyDark:(BOOL)dark {
    self.mapView.appearance =
        [NSAppearance appearanceNamed:dark ? NSAppearanceNameDarkAqua : NSAppearanceNameAqua];
}

- (void)attachToWebView:(WKWebView *)webView frame:(CGRect)frame dark:(BOOL)dark {
    NSView *host = webView.superview;
    if (!host) {
        NSLog(@"[footos] map: web view has no superview, cannot attach");
        return;
    }
    self.webView = webView;
    if (!self.mapView) {
        MKMapView *map = [[MKMapView alloc] initWithFrame:NSZeroRect];
        map.delegate = self;
        map.rotateEnabled = NO;
        map.pitchEnabled = NO;
        map.showsCompass = NO;
        map.showsScale = NO;
        map.showsZoomControls = NO;
        map.showsUserLocation = NO;
        if (@available(macOS 13.0, *)) {
            // A quiet base map with no points of interest: the photos are the content.
            MKStandardMapConfiguration *config =
                [[MKStandardMapConfiguration alloc] initWithEmphasisStyle:MKStandardMapEmphasisStyleMuted];
            config.pointOfInterestFilter = [MKPointOfInterestFilter filterExcludingAllCategories];
            map.preferredConfiguration = config;
        } else {
            map.pointOfInterestFilter = [MKPointOfInterestFilter filterExcludingAllCategories];
        }
        [map registerClass:[FootosAnnotationView class] forAnnotationViewWithReuseIdentifier:@"footos"];
        self.mapView = map;
    }
    [self applyDark:dark];
    if (self.mapView.superview != host) {
        [host addSubview:self.mapView positioned:NSWindowBelow relativeTo:webView];
    }

    // The page leaves the map area unpainted; the web view must not paint it either.
    @try {
        [webView setValue:@NO forKey:@"drawsBackground"];
    } @catch (NSException *e) {
        NSLog(@"[footos] map: could not make the web view transparent: %@", e);
    }
    if (@available(macOS 12.0, *)) {
        webView.underPageBackgroundColor = NSColor.clearColor;
    }
    [self installHitTestOn:webView];

    [self setFrameInWebView:frame];
    NSLog(@"[footos] map attached (gcj=%d, host=%@, web flipped=%d, host flipped=%d, map frame=%@)",
          self.useGCJ, NSStringFromClass([host class]), webView.isFlipped, host.isFlipped,
          NSStringFromRect(self.mapView.frame));
    [self reportRegionMoving:NO];
}

- (void)detach {
    [self.mapView removeAnnotations:self.mapView.annotations];
    [self.mapView removeFromSuperview];
    [self.annotations removeAllObjects];
    self.mapView = nil;
    self.overlaysEnabled = NO;
}

- (void)setFrameInWebView:(CGRect)frame {
    WKWebView *webView = self.webView;
    if (!webView || !self.mapView || frame.size.width <= 0 || frame.size.height <= 0) {
        return;
    }
    self.mapRectInWebView = frame;
    NSRect target = [webView convertRect:[self viewRectFromPage:frame] toView:self.mapView.superview];
    if (!NSEqualRects(self.mapView.frame, target)) {
        self.mapView.frame = target;
    }
}

- (void)setOverlays:(NSDictionary *)body {
    NSMutableArray<NSValue *> *rects = [NSMutableArray array];
    for (NSArray *r in ([body[@"rects"] isKindOfClass:[NSArray class]] ? body[@"rects"] : @[])) {
        if ([r isKindOfClass:[NSArray class]] && r.count == 4) {
            [rects addObject:[NSValue valueWithRect:NSMakeRect([r[0] doubleValue], [r[1] doubleValue],
                                                               [r[2] doubleValue], [r[3] doubleValue])]];
        }
    }
    self.overlayRects = rects;
    self.overlaysEnabled = [body[@"enabled"] boolValue];
}

#pragma mark Mouse routing

- (void)installHitTestOn:(WKWebView *)webView {
    // The method is replaced on the web view's class. Giving this one web view
    // a class made at run time (which is what the iOS side does) is not an
    // option here: AppKit resolves some view properties per class, cannot do
    // so for such a class, and aborts as soon as a pointer moves over the
    // window ("NSDP_getComputedPropertyValue: imp != NULL").
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        // `-class`, not object_getClass: key-value observing may have put its
        // own subclass in front.
        Class cls = [webView class];
        SEL sel = @selector(hitTest:);
        Method inherited = class_getInstanceMethod(cls, sel);
        gOriginalHitTest = method_getImplementation(inherited);
        // Adds an override when the class only inherits the method; replaces
        // the class's own otherwise.
        if (!class_addMethod(cls, sel, (IMP)FMWebViewHitTest, method_getTypeEncoding(inherited))) {
            method_setImplementation(inherited, (IMP)FMWebViewHitTest);
        }
        NSLog(@"[footos] map: hit test installed on %@", NSStringFromClass(cls));
    });
}

/// A point of the web view (page coordinates) that belongs to the map.
- (BOOL)passesThroughPoint:(NSPoint)point {
    if (!self.mapView || !self.overlaysEnabled) {
        return NO;
    }
    if (!NSPointInRect(point, self.mapRectInWebView)) {
        return NO;
    }
    for (NSValue *value in self.overlayRects) {
        if (NSPointInRect(point, value.rectValue)) {
            return NO;
        }
    }
    return YES;
}

#pragma mark Markers

- (void)setFeatures:(NSArray<NSDictionary *> *)features {
    NSMutableSet<NSString *> *seen = [NSMutableSet setWithCapacity:features.count];
    NSMutableArray<FootosAnnotation *> *added = [NSMutableArray array];
    for (NSDictionary *f in features) {
        if (![f isKindOfClass:[NSDictionary class]]) {
            continue;
        }
        NSString *key = [f[@"key"] isKindOfClass:[NSString class]] ? f[@"key"] : nil;
        if (!key) {
            continue;
        }
        [seen addObject:key];
        CLLocationCoordinate2D coordinate =
            [self displayCoordinateForLat:[f[@"lat"] doubleValue] lon:[f[@"lon"] doubleValue]];
        NSInteger count = [f[@"count"] integerValue];
        NSString *label = [f[@"label"] isKindOfClass:[NSString class]] ? f[@"label"] : @"";
        NSString *thumb = [f[@"thumb"] isKindOfClass:[NSString class]] ? f[@"thumb"] : @"";

        FootosAnnotation *existing = self.annotations[key];
        if (!existing) {
            FootosAnnotation *item = [[FootosAnnotation alloc] init];
            item.key = key;
            item.coordinate = coordinate;
            item.count = count;
            item.label = label;
            item.thumb = thumb;
            self.annotations[key] = item;
            [added addObject:item];
            continue;
        }
        BOOL changed = existing.count != count
            || ![existing.thumb isEqualToString:thumb]
            || ![existing.label isEqualToString:label];
        if (existing.coordinate.latitude != coordinate.latitude
            || existing.coordinate.longitude != coordinate.longitude) {
            existing.coordinate = coordinate;
        }
        if (changed) {
            existing.count = count;
            existing.label = label;
            existing.thumb = thumb;
            FootosAnnotationView *view = (FootosAnnotationView *)[self.mapView viewForAnnotation:existing];
            if ([view isKindOfClass:[FootosAnnotationView class]]) {
                [view configure];
            }
        }
    }

    NSMutableArray<FootosAnnotation *> *removed = [NSMutableArray array];
    for (NSString *key in self.annotations.allKeys) {
        if (![seen containsObject:key]) {
            [removed addObject:self.annotations[key]];
            [self.annotations removeObjectForKey:key];
        }
    }
    if (removed.count) {
        [self.mapView removeAnnotations:removed];
    }
    if (added.count) {
        [self.mapView addAnnotations:added];
    }
}

- (MKAnnotationView *)mapView:(MKMapView *)mapView viewForAnnotation:(id<MKAnnotation>)annotation {
    if (![annotation isKindOfClass:[FootosAnnotation class]]) {
        return nil;
    }
    FootosAnnotationView *view =
        (FootosAnnotationView *)[mapView dequeueReusableAnnotationViewWithIdentifier:@"footos" forAnnotation:annotation];
    view.annotation = annotation;
    [view configure];
    return view;
}

- (void)mapView:(MKMapView *)mapView didSelectAnnotationView:(MKAnnotationView *)view {
    FootosAnnotation *item = (FootosAnnotation *)view.annotation;
    [mapView deselectAnnotation:view.annotation animated:NO];
    if (![item isKindOfClass:[FootosAnnotation class]]) {
        return;
    }
    [self send:@"onTap" payload:@{@"key": item.key}];
}

/// The pointer went onto or off a marker: the page shows a preview next to it.
- (void)hoverAnnotationView:(FootosAnnotationView *)view inside:(BOOL)inside {
    FootosAnnotation *item = (FootosAnnotation *)view.annotation;
    WKWebView *webView = self.webView;
    if (![item isKindOfClass:[FootosAnnotation class]] || !webView) {
        return;
    }
    if (!inside) {
        [self send:@"onHoverEnd" payload:@{@"key": item.key}];
        return;
    }
    NSRect inWeb = [webView convertRect:view.bounds fromView:view];
    // The top middle of the marker, in page coordinates.
    NSPoint top = [self pagePointFromView:NSMakePoint(NSMidX(inWeb), webView.isFlipped ? NSMinY(inWeb) : NSMaxY(inWeb))];
    [self send:@"onHover" payload:@{
        @"key": item.key,
        @"x": @(top.x),
        @"y": @(top.y),
        @"size": @(inWeb.size.height),
    }];
}

#pragma mark Camera

- (void)fitBounds:(NSDictionary *)body {
    CLLocationCoordinate2D sw = [self displayCoordinateForLat:[body[@"s"] doubleValue] lon:[body[@"w"] doubleValue]];
    CLLocationCoordinate2D ne = [self displayCoordinateForLat:[body[@"n"] doubleValue] lon:[body[@"e"] doubleValue]];
    MKMapPoint a = MKMapPointForCoordinate(sw);
    MKMapPoint b = MKMapPointForCoordinate(ne);
    MKMapRect rect = MKMapRectMake(MIN(a.x, b.x), MIN(a.y, b.y), fabs(a.x - b.x), fabs(a.y - b.y));
    CGFloat padding = body[@"padding"] ? [body[@"padding"] doubleValue] : 60;
    CGFloat paddingTop = body[@"paddingTop"] ? [body[@"paddingTop"] doubleValue] : padding;
    double maxZoom = body[@"maxZoom"] ? [body[@"maxZoom"] doubleValue] : 18;
    BOOL animated = body[@"animated"] ? [body[@"animated"] boolValue] : YES;

    CGSize size = self.mapView.bounds.size;
    CGFloat usable = MAX(1, size.width - 2 * padding);
    double lonDelta = rect.size.width / MKMapSizeWorld.width * 360.0;
    double zoom = lonDelta > 0 ? log2(360.0 * usable / (kWorldPx * lonDelta)) : 99;
    double minZoom = (body[@"minZoom"] && body[@"minZoom"] != [NSNull null]) ? [body[@"minZoom"] doubleValue] : -1;
    CLLocationCoordinate2D boxCenter =
        MKCoordinateForMapPoint(MKMapPointMake(MKMapRectGetMidX(rect), MKMapRectGetMidY(rect)));
    if (zoom > maxZoom || rect.size.width <= 0) {
        // The box is tiny (or a single point): stop at maxZoom around its centre.
        [self.mapView setVisibleMapRect:[self mapRectForCenter:boxCenter zoom:maxZoom] animated:animated];
        return;
    }
    if (zoom < minZoom) {
        // Fitting the box would not get close enough (a cluster must split
        // when clicked): go to minZoom around its centre instead.
        [self.mapView setVisibleMapRect:[self mapRectForCenter:boxCenter zoom:minZoom] animated:animated];
        return;
    }
    [self.mapView setVisibleMapRect:rect
                        edgePadding:NSEdgeInsetsMake(paddingTop, padding, padding, padding)
                           animated:animated];
}

- (void)easeTo:(NSDictionary *)body {
    CLLocationCoordinate2D center =
        [self displayCoordinateForLat:[body[@"lat"] doubleValue] lon:[body[@"lon"] doubleValue]];
    double zoom = [self currentZoom];
    if (body[@"zoom"] && body[@"zoom"] != [NSNull null]) {
        zoom = [body[@"zoom"] doubleValue];
    }
    if (body[@"minZoom"] && body[@"minZoom"] != [NSNull null]) {
        zoom = MAX(zoom, [body[@"minZoom"] doubleValue]);
    }
    MKMapRect rect = [self mapRectForCenter:center zoom:zoom];
    NSTimeInterval duration = body[@"duration"] ? [body[@"duration"] doubleValue] / 1000.0 : 0.5;
    [self.mapView setVisibleMapRect:rect animated:duration > 0];
}

#pragma mark Region reports

- (void)mapViewDidChangeVisibleRegion:(MKMapView *)mapView {
    // While the camera moves: at most one report per interval, plus a trailing one.
    CFTimeInterval now = CACurrentMediaTime();
    if (now - self.lastRegionReport >= kRegionReportInterval) {
        [self reportRegionMoving:YES];
        return;
    }
    if (self.regionReportPending) {
        return;
    }
    self.regionReportPending = YES;
    __weak typeof(self) weakSelf = self;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(kRegionReportInterval * NSEC_PER_SEC)),
                   dispatch_get_main_queue(), ^{
        if (weakSelf.regionReportPending) {
            [weakSelf reportRegionMoving:YES];
        }
    });
}

- (void)mapView:(MKMapView *)mapView regionDidChangeAnimated:(BOOL)animated {
    [self reportRegionMoving:NO];
}

- (void)reportRegionMoving:(BOOL)moving {
    MKMapView *map = self.mapView;
    if (!map || map.bounds.size.width <= 0) {
        return;
    }
    self.lastRegionReport = CACurrentMediaTime();
    self.regionReportPending = NO;

    MKMapRect rect = map.visibleMapRect;
    double lonDelta = rect.size.width / MKMapSizeWorld.width * 360.0;
    CLLocationCoordinate2D center = [self realCoordinate:map.centerCoordinate];
    CLLocationCoordinate2D top =
        [self realCoordinate:MKCoordinateForMapPoint(MKMapPointMake(MKMapRectGetMidX(rect), MKMapRectGetMinY(rect)))];
    CLLocationCoordinate2D bottom =
        [self realCoordinate:MKCoordinateForMapPoint(MKMapPointMake(MKMapRectGetMidX(rect), MKMapRectGetMaxY(rect)))];

    double west = -180, east = 180;
    if (lonDelta < 360.0) {
        west = center.longitude - lonDelta / 2;
        east = center.longitude + lonDelta / 2;
        // Across the antimeridian the box is given with west > east.
        if (west < -180) west += 360;
        if (east > 180) east -= 360;
    }
    [self send:@"onRegion" payload:@{
        @"west": @(west),
        @"east": @(east),
        @"south": @(MAX(-90, bottom.latitude)),
        @"north": @(MIN(90, top.latitude)),
        @"zoom": @([self zoomForLongitudeDelta:lonDelta]),
        @"lon": @(center.longitude),
        @"lat": @(center.latitude),
        @"moving": @(moving),
    }];
}

- (void)send:(NSString *)handler payload:(NSDictionary *)payload {
    WKWebView *webView = self.webView;
    if (!webView) {
        return;
    }
    NSData *data = [NSJSONSerialization dataWithJSONObject:payload options:0 error:nil];
    NSString *json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] ?: @"{}";
    NSString *js = [NSString stringWithFormat:
        @"window.__footosNativeMap&&window.__footosNativeMap.%@&&window.__footosNativeMap.%@(%@)",
        handler, handler, json];
    [webView evaluateJavaScript:js completionHandler:nil];
}

@end

/// One map command from the page. `webview` is the WKWebView, `json` the
/// command's payload as a JSON object. Main thread only.
void footos_mac_map_command(void *webview, const char *command, const char *json) {
    if (!webview || !command) {
        return;
    }
    WKWebView *webView = (__bridge WKWebView *)webview;
    NSString *name = [NSString stringWithUTF8String:command];
    NSDictionary *body = @{};
    if (json) {
        NSData *data = [NSData dataWithBytes:json length:strlen(json)];
        id parsed = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
        if ([parsed isKindOfClass:[NSDictionary class]]) {
            body = parsed;
        }
    }
    [[FootosMacMap shared] handleCommand:name body:body webView:webView];
}
