#import "FootosMapView.h"

#import <ImageIO/ImageIO.h>
#import <MapKit/MapKit.h>
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

@interface FootosAnnotationView : MKAnnotationView
@property (nonatomic, strong) UIImageView *imageView;
@property (nonatomic, strong) UILabel *countLabel;
@property (nonatomic, copy) NSString *shownThumb;
- (void)configure;
@end

static NSCache<NSString *, UIImage *> *FMThumbCache(void) {
    static NSCache *cache;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        cache = [[NSCache alloc] init];
        cache.countLimit = 300;
    });
    return cache;
}

static UIImage *FMLoadThumb(NSString *path) {
    NSURL *url = [NSURL fileURLWithPath:path];
    CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)url, NULL);
    if (!source) {
        return nil;
    }
    NSDictionary *options = @{
        (id)kCGImageSourceCreateThumbnailFromImageAlways: @YES,
        (id)kCGImageSourceCreateThumbnailWithTransform: @YES,
        (id)kCGImageSourceThumbnailMaxPixelSize: @(kClusterSize * 3),
    };
    CGImageRef cg = CGImageSourceCreateThumbnailAtIndex(source, 0, (__bridge CFDictionaryRef)options);
    CFRelease(source);
    if (!cg) {
        return nil;
    }
    UIImage *image = [UIImage imageWithCGImage:cg];
    CGImageRelease(cg);
    return image;
}

@implementation FootosAnnotationView

- (instancetype)initWithAnnotation:(id<MKAnnotation>)annotation reuseIdentifier:(NSString *)reuseIdentifier {
    self = [super initWithAnnotation:annotation reuseIdentifier:reuseIdentifier];
    if (self) {
        self.backgroundColor = UIColor.clearColor;
        self.collisionMode = MKAnnotationViewCollisionModeNone;
        self.displayPriority = MKFeatureDisplayPriorityRequired;
        self.canShowCallout = NO;

        _imageView = [[UIImageView alloc] init];
        _imageView.contentMode = UIViewContentModeScaleAspectFill;
        _imageView.clipsToBounds = YES;
        _imageView.backgroundColor = [UIColor colorWithWhite:0.75 alpha:1];
        _imageView.layer.borderColor = UIColor.whiteColor.CGColor;
        _imageView.layer.borderWidth = 2;
        [self addSubview:_imageView];

        _countLabel = [[UILabel alloc] init];
        _countLabel.font = [UIFont systemFontOfSize:13 weight:UIFontWeightBold];
        _countLabel.textColor = UIColor.whiteColor;
        _countLabel.textAlignment = NSTextAlignmentCenter;
        _countLabel.backgroundColor = [UIColor colorWithWhite:0 alpha:0.6];
        _countLabel.layer.cornerRadius = 9;
        _countLabel.layer.masksToBounds = YES;
        [self addSubview:_countLabel];

        self.layer.shadowColor = UIColor.blackColor.CGColor;
        self.layer.shadowOpacity = 0.35;
        self.layer.shadowRadius = 3;
        self.layer.shadowOffset = CGSizeMake(0, 1);
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
    self.bounds = CGRectMake(0, 0, size, size);
    self.imageView.frame = self.bounds;
    self.imageView.layer.cornerRadius = size / 2;
    self.layer.shadowPath = [UIBezierPath bezierPathWithOvalInRect:self.bounds].CGPath;
    // Bigger clusters sit above smaller ones and above single photos.
    if (@available(iOS 14.0, *)) {
        self.zPriority = (MKAnnotationViewZPriority)MIN(999, 500 + item.count / 10);
    }

    self.countLabel.hidden = !cluster;
    if (cluster) {
        self.countLabel.text = item.label.length ? item.label : [NSString stringWithFormat:@"%ld", (long)item.count];
        CGSize fit = [self.countLabel sizeThatFits:CGSizeMake(100, 18)];
        CGFloat w = MAX(26, fit.width + 12);
        self.countLabel.frame = CGRectMake((size - w) / 2, (size - 18) / 2, w, 18);
    }

    NSString *thumb = item.thumb ?: @"";
    if ([thumb isEqualToString:self.shownThumb ?: @""] && self.imageView.image) {
        return;
    }
    self.shownThumb = thumb;
    UIImage *cached = thumb.length ? [FMThumbCache() objectForKey:thumb] : nil;
    self.imageView.image = cached;
    if (cached || thumb.length == 0) {
        return;
    }
    __weak typeof(self) weakSelf = self;
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        UIImage *image = FMLoadThumb(thumb);
        if (!image) {
            return;
        }
        [FMThumbCache() setObject:image forKey:thumb];
        dispatch_async(dispatch_get_main_queue(), ^{
            if ([weakSelf.shownThumb isEqualToString:thumb]) {
                weakSelf.imageView.image = image;
            }
        });
    });
}

- (void)prepareForReuse {
    [super prepareForReuse];
    self.shownThumb = nil;
    self.imageView.image = nil;
}

@end

#pragma mark - Map

@interface FootosMapView () <MKMapViewDelegate>
@property (nonatomic, strong, nullable) MKMapView *mapView;
@property (nonatomic, weak, nullable) WKWebView *webView;
@property (nonatomic, copy, nullable) void (^eval)(NSString *js);
@property (nonatomic, strong) NSMutableDictionary<NSString *, FootosAnnotation *> *annotations;
/// Where the web UI lies over the map, in web view coordinates.
@property (nonatomic, copy) NSArray<NSValue *> *overlayRects;
@property (nonatomic, assign) BOOL overlaysEnabled;
@property (nonatomic, assign) CGRect mapRectInWebView;
@property (nonatomic, assign) BOOL useGCJ;
@property (nonatomic, assign) CFTimeInterval lastRegionReport;
@property (nonatomic, assign) BOOL regionReportPending;
- (BOOL)passesThroughPoint:(CGPoint)point;
- (void)reportRegionMoving:(BOOL)moving;
@end

// Installed on the web view (as a runtime subclass): a touch on the map area
// that no web control covers is not the web view's, so the hit test falls
// through to the map view underneath.
static UIView *FMWebViewHitTest(UIView *self, SEL _cmd, CGPoint point, UIEvent *event) {
    if ([[FootosMapView shared] passesThroughPoint:point]) {
        return nil;
    }
    struct objc_super sup = { self, class_getSuperclass(object_getClass(self)) };
    return ((UIView *(*)(struct objc_super *, SEL, CGPoint, UIEvent *))objc_msgSendSuper)(&sup, _cmd, point, event);
}

@implementation FootosMapView

+ (instancetype)shared {
    static FootosMapView *instance;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        instance = [[FootosMapView alloc] init];
    });
    return instance;
}

- (instancetype)init {
    self = [super init];
    if (self) {
        _annotations = [NSMutableDictionary dictionary];
        _overlayRects = @[];
        _overlaysEnabled = NO;
        // No public API tells whether the map is being drawn in GCJ-02. Apple
        // serves the shifted (AutoNavi) map to devices that are in mainland
        // China, and the plain WGS-84 map everywhere else, including for
        // places in China. The time zone is the closest public stand-in for
        // "the device is in mainland China". The region setting is not: a
        // phone set to zh_CN but used abroad gets the unshifted map, and
        // converting there is what puts every photo a few hundred metres off.
        NSString *zone = [NSTimeZone localTimeZone].name ?: @"";
        NSSet<NSString *> *mainland = [NSSet setWithObjects:
            @"Asia/Shanghai", @"Asia/Urumqi", @"Asia/Chongqing", @"Asia/Harbin", @"Asia/Kashgar", nil];
        _useGCJ = [mainland containsObject:zone];
        NSLog(@"[footos] map: time zone %@, gcj=%d", zone, _useGCJ);
    }
    return self;
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
    double lonDelta = rect.size.width / MKMapSizeWorld.width * 360.0;
    return [self zoomForLongitudeDelta:lonDelta];
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

- (BOOL)handleCommand:(NSString *)command
                 body:(NSDictionary *)body
              webView:(WKWebView *)webView
                 eval:(void (^)(NSString *js))eval {
    if (![command hasPrefix:@"map"]) {
        return NO;
    }
    self.eval = eval;
    if ([command isEqualToString:@"mapAttach"]) {
        [self attachToWebView:webView frame:[self rectFrom:body] dark:[body[@"dark"] boolValue]];
    } else if ([command isEqualToString:@"mapDetach"]) {
        [self detach];
    } else if (!self.mapView) {
        return YES;
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
        self.mapView.overrideUserInterfaceStyle = [body[@"dark"] boolValue]
            ? UIUserInterfaceStyleDark : UIUserInterfaceStyleLight;
    } else if ([command isEqualToString:@"mapSetOverlays"]) {
        [self setOverlays:body];
    }
    return YES;
}

- (CGRect)rectFrom:(NSDictionary *)body {
    return CGRectMake([body[@"x"] doubleValue], [body[@"y"] doubleValue],
                      [body[@"w"] doubleValue], [body[@"h"] doubleValue]);
}

- (void)attachToWebView:(WKWebView *)webView frame:(CGRect)frame dark:(BOOL)dark {
    UIView *host = webView.superview;
    if (!host) {
        NSLog(@"[footos] map: web view has no superview, cannot attach");
        return;
    }
    self.webView = webView;
    if (!self.mapView) {
        MKMapView *map = [[MKMapView alloc] initWithFrame:CGRectZero];
        map.delegate = self;
        map.rotateEnabled = NO;
        map.pitchEnabled = NO;
        map.showsCompass = NO;
        map.showsScale = NO;
        map.showsUserLocation = NO;
        // Camera requests are fitted into the view minus its layout margins.
        // With the safe area folded in, every "show this rect" came out about
        // 0.15 zoom levels short of what was asked for.
        map.insetsLayoutMarginsFromSafeArea = NO;
        map.layoutMargins = UIEdgeInsetsZero;
        if (@available(iOS 16.0, *)) {
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
    self.mapView.overrideUserInterfaceStyle = dark ? UIUserInterfaceStyleDark : UIUserInterfaceStyleLight;
    if (self.mapView.superview != host) {
        [host insertSubview:self.mapView belowSubview:webView];
    }

    // The page leaves the map area unpainted; the web view must not paint it either.
    webView.opaque = NO;
    webView.backgroundColor = UIColor.clearColor;
    webView.scrollView.backgroundColor = UIColor.clearColor;
    [self installHitTestOn:webView];

    [self setFrameInWebView:frame];
    NSLog(@"[footos] map attached (gcj=%d, host=%@)", self.useGCJ, NSStringFromClass([host class]));
    [self reportRegionMoving:NO];
}

- (void)detach {
    [self.mapView removeFromSuperview];
    [self.mapView removeAnnotations:self.mapView.annotations];
    [self.annotations removeAllObjects];
    self.mapView = nil;
    self.overlaysEnabled = NO;
    WKWebView *webView = self.webView;
    if (webView) {
        webView.opaque = YES;
        webView.backgroundColor = UIColor.blackColor;
    }
}

- (void)setFrameInWebView:(CGRect)frame {
    WKWebView *webView = self.webView;
    if (!webView || !self.mapView || frame.size.width <= 0 || frame.size.height <= 0) {
        return;
    }
    self.mapRectInWebView = frame;
    CGRect target = [webView convertRect:frame toView:self.mapView.superview];
    if (!CGRectEqualToRect(self.mapView.frame, target)) {
        self.mapView.frame = target;
    }
}

- (void)setOverlays:(NSDictionary *)body {
    NSMutableArray<NSValue *> *rects = [NSMutableArray array];
    for (NSArray *r in ([body[@"rects"] isKindOfClass:[NSArray class]] ? body[@"rects"] : @[])) {
        if ([r isKindOfClass:[NSArray class]] && r.count == 4) {
            [rects addObject:[NSValue valueWithCGRect:CGRectMake([r[0] doubleValue], [r[1] doubleValue],
                                                                 [r[2] doubleValue], [r[3] doubleValue])]];
        }
    }
    self.overlayRects = rects;
    self.overlaysEnabled = [body[@"enabled"] boolValue];
}

#pragma mark Touch routing

- (void)installHitTestOn:(WKWebView *)webView {
    static const char *kSuffix = "_FootosPassthrough";
    Class current = object_getClass(webView);
    NSString *name = NSStringFromClass(current);
    if ([name hasSuffix:@(kSuffix)]) {
        return;
    }
    NSString *subName = [name stringByAppendingString:@(kSuffix)];
    Class sub = NSClassFromString(subName);
    if (!sub) {
        sub = objc_allocateClassPair(current, subName.UTF8String, 0);
        if (!sub) {
            NSLog(@"[footos] map: could not subclass the web view; touches will not reach the map");
            return;
        }
        Method original = class_getInstanceMethod(current, @selector(hitTest:withEvent:));
        class_addMethod(sub, @selector(hitTest:withEvent:), (IMP)FMWebViewHitTest, method_getTypeEncoding(original));
        objc_registerClassPair(sub);
    }
    object_setClass(webView, sub);
}

/// A point of the web view (its own coordinates) that belongs to the map.
- (BOOL)passesThroughPoint:(CGPoint)point {
    if (!self.mapView || !self.overlaysEnabled) {
        return NO;
    }
    if (!CGRectContainsPoint(self.mapRectInWebView, point)) {
        return NO;
    }
    for (NSValue *value in self.overlayRects) {
        if (CGRectContainsPoint(value.CGRectValue, point)) {
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
    NSData *data = [NSJSONSerialization dataWithJSONObject:@{@"key": item.key} options:0 error:nil];
    NSString *json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    [self send:[NSString stringWithFormat:@"window.__footosNativeMap&&window.__footosNativeMap.onTap(%@)", json]];
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
        // when tapped): go to minZoom around its centre instead.
        [self.mapView setVisibleMapRect:[self mapRectForCenter:boxCenter zoom:minZoom] animated:animated];
        return;
    }
    [self.mapView setVisibleMapRect:rect
                        edgePadding:UIEdgeInsetsMake(paddingTop, padding, padding, padding)
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
    if (duration <= 0) {
        [self.mapView setVisibleMapRect:rect animated:NO];
        return;
    }
    [UIView animateWithDuration:duration
                          delay:0
                        options:UIViewAnimationOptionCurveEaseInOut
                     animations:^{
        [self.mapView setVisibleMapRect:rect animated:YES];
    } completion:nil];
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
    NSDictionary *payload = @{
        @"west": @(west),
        @"east": @(east),
        @"south": @(MAX(-90, bottom.latitude)),
        @"north": @(MIN(90, top.latitude)),
        @"zoom": @([self zoomForLongitudeDelta:lonDelta]),
        @"lon": @(center.longitude),
        @"lat": @(center.latitude),
        @"moving": @(moving),
    };
    NSData *data = [NSJSONSerialization dataWithJSONObject:payload options:0 error:nil];
    NSString *json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    [self send:[NSString stringWithFormat:@"window.__footosNativeMap&&window.__footosNativeMap.onRegion(%@)", json]];
}

- (void)send:(NSString *)js {
    if (self.eval) {
        self.eval(js);
    }
}

@end
