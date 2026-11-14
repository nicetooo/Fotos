#import "FootosPhotoViewer.h"
#import <AVFoundation/AVFoundation.h>
#import <ImageIO/ImageIO.h>
#import <Photos/Photos.h>
#import <PhotosUI/PhotosUI.h>

// Native photo viewer modelled on the system Photos app: paging, pinch and
// double-tap zoom, swipe down to dismiss, tap to toggle chrome, a thumbnail
// scrubber, long-press Live Photos and inline video.

static NSString *const kIOSPhotoPrefix = @"ios-photo://";
static const CGFloat kPageSpacing = 16.0;
static const CGFloat kThumbW = 36.0;
static const CGFloat kThumbH = 48.0;
static const CGFloat kThumbGap = 2.0;
static const CGFloat kStripHeight = 56.0;
static const CGFloat kMaxZoom = 4.0;
/// Longest side of the image loaded for zooming. A 24 MP photo fits whole;
/// larger ones are scaled to this, which keeps one decoded image near 110 MB.
static const CGFloat kZoomMaxPixel = 6048.0;
static const CGFloat kDoubleTapZoom = 2.5;
static const CGFloat kDismissDistance = 110.0;
static const CGFloat kDismissVelocity = 900.0;

#pragma mark - Helpers

static NSString *FVString(NSDictionary *item, NSString *key) {
    id value = item[key];
    return [value isKindOfClass:[NSString class]] ? value : @"";
}

/// The PHAsset behind an `ios-photo://` path, or nil for plain files.
static PHAsset *FVAsset(NSString *path) {
    if (![path hasPrefix:kIOSPhotoPrefix]) {
        return nil;
    }
    NSString *identifier = [path substringFromIndex:kIOSPhotoPrefix.length];
    return [PHAsset fetchAssetsWithLocalIdentifiers:@[identifier] options:nil].firstObject;
}

/// Decode a file no larger than `maxPixel` on its long side, without loading
/// the full bitmap. Safe to call off the main thread.
static UIImage *FVDownsample(NSString *file, CGFloat maxPixel) {
    if (file.length == 0) {
        return nil;
    }
    NSURL *url = [NSURL fileURLWithPath:file];
    CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)url, NULL);
    if (!source) {
        return nil;
    }
    NSDictionary *options = @{
        (id)kCGImageSourceCreateThumbnailFromImageAlways: @YES,
        (id)kCGImageSourceCreateThumbnailWithTransform: @YES,
        (id)kCGImageSourceShouldCacheImmediately: @YES,
        (id)kCGImageSourceThumbnailMaxPixelSize: @(maxPixel),
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

static NSCache<NSString *, UIImage *> *FVThumbCache(void) {
    static NSCache *cache;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        cache = [[NSCache alloc] init];
        cache.countLimit = 400;
    });
    return cache;
}

static dispatch_queue_t FVDecodeQueue(void) {
    static dispatch_queue_t queue;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        queue = dispatch_queue_create("app.footos.viewer.decode", DISPATCH_QUEUE_CONCURRENT);
    });
    return queue;
}

/// Small thumbnail for the scrubber and as the first frame of a page.
static void FVLoadThumb(NSString *file, void (^done)(UIImage *_Nullable image)) {
    if (file.length == 0) {
        done(nil);
        return;
    }
    UIImage *cached = [FVThumbCache() objectForKey:file];
    if (cached) {
        done(cached);
        return;
    }
    dispatch_async(FVDecodeQueue(), ^{
        UIImage *image = FVDownsample(file, 256);
        if (image) {
            [FVThumbCache() setObject:image forKey:file];
        }
        dispatch_async(dispatch_get_main_queue(), ^{
            done(image);
        });
    });
}

#pragma mark - Page

/// The default audio session is silenced by the ring/silent switch. Media the
/// user chose to play (a video, a Live Photo held down) should be heard.
static void FVActivatePlaybackAudio(void) {
    AVAudioSession *session = [AVAudioSession sharedInstance];
    if (![session.category isEqualToString:AVAudioSessionCategoryPlayback]) {
        [session setCategory:AVAudioSessionCategoryPlayback error:nil];
    }
    [session setActive:YES error:nil];
}

@interface FootosPhotoPage : UIViewController <UIScrollViewDelegate>
@property (nonatomic, strong) NSDictionary *item;
@property (nonatomic, assign) NSInteger index;
@property (nonatomic, strong) UIScrollView *scrollView;
@property (nonatomic, strong) UIImageView *imageView;
@property (nonatomic, strong, nullable) PHAsset *asset;
@property (nonatomic, strong, nullable) UIView *liveView;
@property (nonatomic, strong, nullable) AVPlayer *livePlayer;
@property (nonatomic, strong, nullable) AVPlayerLayer *livePlayerLayer;
@property (nonatomic, assign) BOOL liveLoading;
@property (nonatomic, strong, nullable) AVPlayer *player;
@property (nonatomic, strong, nullable) AVPlayerLayer *playerLayer;
@property (nonatomic, strong) UIView *videoView;
@property (nonatomic, strong) UIImageView *playGlyph;
@property (nonatomic, strong) UILabel *liveBadge;
@property (nonatomic, assign) PHImageRequestID imageRequest;
@property (nonatomic, assign) BOOL hasFullImage;
/// The full-resolution image was asked for (the user zoomed in).
@property (nonatomic, assign) BOOL wantsOriginal;
@property (nonatomic, assign) BOOL hasOriginal;
@property (nonatomic, assign) PHImageRequestID originalRequest;
@property (nonatomic, assign) BOOL livePressed;
@property (nonatomic, strong, nullable) id playbackObserver;
@end

@implementation FootosPhotoPage

- (BOOL)isVideo {
    return [FVString(self.item, @"media_type") isEqualToString:@"video"];
}

- (BOOL)isLivePhoto {
    if (self.asset) {
        return (self.asset.mediaSubtypes & PHAssetMediaSubtypePhotoLive) != 0;
    }
    return NO;
}

- (BOOL)isZoomed {
    return self.scrollView.zoomScale > self.scrollView.minimumZoomScale + 0.01;
}

- (BOOL)isPlaying {
    return self.player && self.player.rate > 0;
}

- (void)viewDidLoad {
    [super viewDidLoad];
    self.view.backgroundColor = UIColor.clearColor;
    self.imageRequest = PHInvalidImageRequestID;
    self.originalRequest = PHInvalidImageRequestID;
    self.asset = FVAsset(FVString(self.item, @"path"));

    self.scrollView = [[UIScrollView alloc] initWithFrame:self.view.bounds];
    self.scrollView.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
    self.scrollView.delegate = self;
    self.scrollView.minimumZoomScale = 1.0;
    self.scrollView.maximumZoomScale = [self isVideo] ? 1.0 : kMaxZoom;
    self.scrollView.showsVerticalScrollIndicator = NO;
    self.scrollView.showsHorizontalScrollIndicator = NO;
    self.scrollView.contentInsetAdjustmentBehavior = UIScrollViewContentInsetAdjustmentNever;
    self.scrollView.decelerationRate = UIScrollViewDecelerationRateFast;
    [self.view addSubview:self.scrollView];

    self.imageView = [[UIImageView alloc] initWithFrame:self.scrollView.bounds];
    self.imageView.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
    self.imageView.contentMode = UIViewContentModeScaleAspectFit;
    self.imageView.clipsToBounds = YES;
    [self.scrollView addSubview:self.imageView];

    self.videoView = [[UIView alloc] initWithFrame:self.view.bounds];
    self.videoView.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
    self.videoView.userInteractionEnabled = NO;
    self.videoView.hidden = YES;
    [self.view addSubview:self.videoView];

    UIImageSymbolConfiguration *config = [UIImageSymbolConfiguration configurationWithPointSize:56 weight:UIImageSymbolWeightRegular];
    self.playGlyph = [[UIImageView alloc] initWithImage:[UIImage systemImageNamed:@"play.circle.fill" withConfiguration:config]];
    self.playGlyph.tintColor = [UIColor colorWithWhite:1 alpha:0.92];
    self.playGlyph.hidden = ![self isVideo];
    self.playGlyph.layer.shadowColor = UIColor.blackColor.CGColor;
    self.playGlyph.layer.shadowOpacity = 0.35;
    self.playGlyph.layer.shadowRadius = 8;
    self.playGlyph.layer.shadowOffset = CGSizeZero;
    [self.view addSubview:self.playGlyph];

    self.liveBadge = [[UILabel alloc] init];
    self.liveBadge.text = @"  LIVE  ";
    self.liveBadge.font = [UIFont systemFontOfSize:11 weight:UIFontWeightSemibold];
    self.liveBadge.textColor = UIColor.whiteColor;
    self.liveBadge.backgroundColor = [UIColor colorWithWhite:0 alpha:0.45];
    self.liveBadge.layer.cornerRadius = 9;
    self.liveBadge.layer.masksToBounds = YES;
    self.liveBadge.hidden = ![self isLivePhoto];
    [self.view addSubview:self.liveBadge];

    [self loadImage];
}

- (void)viewDidLayoutSubviews {
    [super viewDidLayoutSubviews];
    CGRect bounds = self.view.bounds;
    if (![self isZoomed]) {
        self.scrollView.contentSize = bounds.size;
        self.imageView.frame = bounds;
    }
    self.playerLayer.frame = self.videoView.bounds;
    self.liveView.frame = bounds;
    self.livePlayerLayer.frame = bounds;
    [self.playGlyph sizeToFit];
    self.playGlyph.center = CGPointMake(CGRectGetMidX(bounds), CGRectGetMidY(bounds));
    CGSize badge = [self.liveBadge sizeThatFits:CGSizeMake(200, 18)];
    self.liveBadge.frame = CGRectMake(16, self.view.safeAreaInsets.top + 56, badge.width, 18);
}

- (void)viewDidDisappear:(BOOL)animated {
    [super viewDidDisappear:animated];
    [self stopPlayback];
    [self stopLivePhoto];
    [self.scrollView setZoomScale:1.0 animated:NO];
    [self dropOriginal];
}

- (void)dealloc {
    if (_imageRequest != PHInvalidImageRequestID) {
        [[PHImageManager defaultManager] cancelImageRequest:_imageRequest];
    }
    if (_originalRequest != PHInvalidImageRequestID) {
        [[PHImageManager defaultManager] cancelImageRequest:_originalRequest];
    }
    if (_playbackObserver) {
        [[NSNotificationCenter defaultCenter] removeObserver:_playbackObserver];
    }
}

#pragma mark Image

/// Thumbnail first for an instant frame, then a screen-sized image.
- (void)loadImage {
    __weak typeof(self) weakSelf = self;
    FVLoadThumb(FVString(self.item, @"thumb"), ^(UIImage *thumb) {
        if (thumb && !weakSelf.hasFullImage) {
            weakSelf.imageView.image = thumb;
        }
    });

    CGFloat scale = UIScreen.mainScreen.scale;
    CGSize screen = UIScreen.mainScreen.bounds.size;
    CGFloat maxPixel = MAX(screen.width, screen.height) * scale;

    if (self.asset) {
        PHImageRequestOptions *options = [[PHImageRequestOptions alloc] init];
        options.deliveryMode = PHImageRequestOptionsDeliveryModeOpportunistic;
        options.resizeMode = PHImageRequestOptionsResizeModeFast;
        options.networkAccessAllowed = YES;
        self.imageRequest = [[PHImageManager defaultManager]
            requestImageForAsset:self.asset
                      targetSize:CGSizeMake(screen.width * scale, screen.height * scale)
                     contentMode:PHImageContentModeAspectFit
                         options:options
                   resultHandler:^(UIImage *image, NSDictionary *info) {
            if (!image) {
                return;
            }
            dispatch_async(dispatch_get_main_queue(), ^{
                weakSelf.hasFullImage = YES;
                if (!weakSelf.hasOriginal) {
                    weakSelf.imageView.image = image;
                }
            });
        }];
        return;
    }

    if ([self isVideo]) {
        return; // the poster is the thumbnail; the player draws the rest
    }
    NSString *file = FVString(self.item, @"path");
    dispatch_async(FVDecodeQueue(), ^{
        UIImage *image = FVDownsample(file, maxPixel);
        if (!image) {
            return;
        }
        dispatch_async(dispatch_get_main_queue(), ^{
            weakSelf.hasFullImage = YES;
            if (!weakSelf.hasOriginal) {
                weakSelf.imageView.image = image;
            }
        });
    });
}

/// The screen-sized image is all that is needed until the user zooms in;
/// then the picture is loaded at its own resolution and swapped in.
- (void)loadOriginal {
    if (self.wantsOriginal || [self isVideo]) {
        return;
    }
    self.wantsOriginal = YES;
    __weak typeof(self) weakSelf = self;
    void (^show)(UIImage *) = ^(UIImage *image) {
        dispatch_async(dispatch_get_main_queue(), ^{
            // Zoomed back out or swiped away in the meantime.
            if (!image || !weakSelf.wantsOriginal) {
                return;
            }
            weakSelf.hasOriginal = YES;
            weakSelf.imageView.image = image;
        });
    };

    if (self.asset) {
        CGFloat width = (CGFloat)MAX(self.asset.pixelWidth, 1);
        CGFloat height = (CGFloat)MAX(self.asset.pixelHeight, 1);
        CGFloat longSide = MAX(width, height);
        if (longSide > kZoomMaxPixel) {
            width *= kZoomMaxPixel / longSide;
            height *= kZoomMaxPixel / longSide;
        }
        PHImageRequestOptions *options = [[PHImageRequestOptions alloc] init];
        options.deliveryMode = PHImageRequestOptionsDeliveryModeHighQualityFormat;
        options.resizeMode = PHImageRequestOptionsResizeModeExact;
        options.networkAccessAllowed = YES;
        self.originalRequest = [[PHImageManager defaultManager]
            requestImageForAsset:self.asset
                      targetSize:CGSizeMake(width, height)
                     contentMode:PHImageContentModeAspectFit
                         options:options
                   resultHandler:^(UIImage *image, NSDictionary *info) {
            if ([info[PHImageResultIsDegradedKey] boolValue]) {
                return;
            }
            show(image);
        }];
        return;
    }
    NSString *file = FVString(self.item, @"path");
    dispatch_async(FVDecodeQueue(), ^{
        show(FVDownsample(file, kZoomMaxPixel));
    });
}

/// Back to the screen-sized image, so a page that is off screen does not
/// keep a full-resolution picture in memory.
- (void)dropOriginal {
    if (!self.wantsOriginal) {
        return;
    }
    self.wantsOriginal = NO;
    if (self.originalRequest != PHInvalidImageRequestID) {
        [[PHImageManager defaultManager] cancelImageRequest:self.originalRequest];
        self.originalRequest = PHInvalidImageRequestID;
    }
    if (self.hasOriginal) {
        self.hasOriginal = NO;
        self.hasFullImage = NO;
        [self loadImage];
    }
}

#pragma mark Zoom

- (void)scrollViewDidZoom:(UIScrollView *)scrollView {
    if (scrollView.zoomScale > scrollView.minimumZoomScale + 0.05) {
        [self loadOriginal];
    }
}

- (UIView *)viewForZoomingInScrollView:(UIScrollView *)scrollView {
    return self.imageView;
}

- (void)toggleZoomAtPoint:(CGPoint)pointInView {
    if ([self isVideo]) {
        return;
    }
    if ([self isZoomed]) {
        [self.scrollView setZoomScale:1.0 animated:YES];
        return;
    }
    CGPoint p = [self.view convertPoint:pointInView toView:self.imageView];
    CGSize size = self.scrollView.bounds.size;
    CGFloat w = size.width / kDoubleTapZoom;
    CGFloat h = size.height / kDoubleTapZoom;
    [self.scrollView zoomToRect:CGRectMake(p.x - w / 2, p.y - h / 2, w, h) animated:YES];
}

#pragma mark Video

- (void)attachPlayerItem:(AVPlayerItem *)item {
    if (!item || self.player) {
        return;
    }
    self.player = [AVPlayer playerWithPlayerItem:item];
    self.playerLayer = [AVPlayerLayer playerLayerWithPlayer:self.player];
    self.playerLayer.videoGravity = AVLayerVideoGravityResizeAspect;
    self.playerLayer.frame = self.videoView.bounds;
    [self.videoView.layer addSublayer:self.playerLayer];
    __weak typeof(self) weakSelf = self;
    self.playbackObserver = [[NSNotificationCenter defaultCenter]
        addObserverForName:AVPlayerItemDidPlayToEndTimeNotification
                    object:item
                     queue:NSOperationQueue.mainQueue
                usingBlock:^(NSNotification *note) {
        [weakSelf.player seekToTime:kCMTimeZero];
        weakSelf.playGlyph.hidden = NO;
    }];
}

/// Returns YES when playback started (or will start once the item is ready).
- (BOOL)togglePlayback {
    if (![self isVideo]) {
        return NO;
    }
    if ([self isPlaying]) {
        [self.player pause];
        self.playGlyph.hidden = NO;
        return NO;
    }
    self.videoView.hidden = NO;
    self.playGlyph.hidden = YES;
    FVActivatePlaybackAudio();
    if (self.player) {
        [self.player play];
        return YES;
    }
    __weak typeof(self) weakSelf = self;
    if (self.asset) {
        PHVideoRequestOptions *options = [[PHVideoRequestOptions alloc] init];
        options.networkAccessAllowed = YES;
        options.deliveryMode = PHVideoRequestOptionsDeliveryModeAutomatic;
        [[PHImageManager defaultManager] requestPlayerItemForVideo:self.asset
                                                           options:options
                                                     resultHandler:^(AVPlayerItem *item, NSDictionary *info) {
            dispatch_async(dispatch_get_main_queue(), ^{
                [weakSelf attachPlayerItem:item];
                [weakSelf.player play];
            });
        }];
    } else {
        NSURL *url = [NSURL fileURLWithPath:FVString(self.item, @"path")];
        [self attachPlayerItem:[AVPlayerItem playerItemWithURL:url]];
        [self.player play];
    }
    return YES;
}

- (void)stopPlayback {
    [self.player pause];
    [self.player seekToTime:kCMTimeZero];
    self.videoView.hidden = YES;
    self.playGlyph.hidden = ![self isVideo];
}

#pragma mark Live Photo

// The paired video is played with our own AVPlayer rather than PHLivePhotoView:
// that view stayed silent with the ring/silent switch on even after the app's
// audio session was set to playback, while AVPlayer follows the session.

/// Where the paired video of this asset is cached.
- (NSString *)liveVideoPath {
    NSString *dir = [NSTemporaryDirectory() stringByAppendingPathComponent:@"footos-live"];
    [[NSFileManager defaultManager] createDirectoryAtPath:dir withIntermediateDirectories:YES attributes:nil error:nil];
    NSString *name = [self.asset.localIdentifier stringByReplacingOccurrencesOfString:@"/" withString:@"_"];
    return [dir stringByAppendingPathComponent:[name stringByAppendingPathExtension:@"mov"]];
}

- (void)attachLiveVideo:(NSString *)path {
    if (self.livePlayer) {
        return;
    }
    self.livePlayer = [AVPlayer playerWithURL:[NSURL fileURLWithPath:path]];
    self.livePlayer.actionAtItemEnd = AVPlayerActionAtItemEndPause;
    self.liveView = [[UIView alloc] initWithFrame:self.view.bounds];
    self.liveView.userInteractionEnabled = NO;
    self.liveView.backgroundColor = UIColor.clearColor;
    self.liveView.alpha = 0;
    self.livePlayerLayer = [AVPlayerLayer playerLayerWithPlayer:self.livePlayer];
    self.livePlayerLayer.videoGravity = AVLayerVideoGravityResizeAspect;
    self.livePlayerLayer.frame = self.liveView.bounds;
    [self.liveView.layer addSublayer:self.livePlayerLayer];
    [self.view insertSubview:self.liveView aboveSubview:self.scrollView];
}

- (void)playLiveVideo {
    if (!self.livePlayer || !self.livePressed) {
        return;
    }
    FVActivatePlaybackAudio();
    [self.livePlayer seekToTime:kCMTimeZero toleranceBefore:kCMTimeZero toleranceAfter:kCMTimeZero];
    [self.livePlayer play];
    [UIView animateWithDuration:0.15 animations:^{
        self.liveView.alpha = 1;
    }];
}

- (void)startLivePhoto {
    if (![self isLivePhoto] || [self isZoomed]) {
        return;
    }
    self.livePressed = YES;
    if (self.livePlayer) {
        [self playLiveVideo];
        return;
    }
    if (self.liveLoading) {
        return;
    }
    NSString *path = [self liveVideoPath];
    if ([[NSFileManager defaultManager] fileExistsAtPath:path]) {
        [self attachLiveVideo:path];
        [self playLiveVideo];
        return;
    }
    PHAssetResource *resource = nil;
    for (PHAssetResource *candidate in [PHAssetResource assetResourcesForAsset:self.asset]) {
        if (candidate.type == PHAssetResourceTypePairedVideo) {
            resource = candidate;
            break;
        }
        if (candidate.type == PHAssetResourceTypeFullSizePairedVideo && !resource) {
            resource = candidate;
        }
    }
    if (!resource) {
        return;
    }
    self.liveLoading = YES;
    PHAssetResourceRequestOptions *options = [[PHAssetResourceRequestOptions alloc] init];
    options.networkAccessAllowed = YES;
    // Write beside the final name, so a half-written file is never played.
    NSString *partial = [path stringByAppendingString:@".part"];
    [[NSFileManager defaultManager] removeItemAtPath:partial error:nil];
    __weak typeof(self) weakSelf = self;
    [[PHAssetResourceManager defaultManager] writeDataForAssetResource:resource
                                                                toFile:[NSURL fileURLWithPath:partial]
                                                               options:options
                                                     completionHandler:^(NSError *error) {
        if (!error) {
            [[NSFileManager defaultManager] removeItemAtPath:path error:nil];
            [[NSFileManager defaultManager] moveItemAtPath:partial toPath:path error:nil];
        }
        dispatch_async(dispatch_get_main_queue(), ^{
            weakSelf.liveLoading = NO;
            if (error || ![[NSFileManager defaultManager] fileExistsAtPath:path]) {
                return;
            }
            [weakSelf attachLiveVideo:path];
            [weakSelf playLiveVideo];
        });
    }];
}

- (void)stopLivePhoto {
    self.livePressed = NO;
    [self.livePlayer pause];
    UIView *view = self.liveView;
    [UIView animateWithDuration:0.15 animations:^{
        view.alpha = 0;
    }];
}

@end

#pragma mark - Scrubber cell

@interface FootosThumbCell : UICollectionViewCell
@property (nonatomic, strong) UIImageView *imageView;
@property (nonatomic, copy) NSString *file;
@end

@implementation FootosThumbCell

- (instancetype)initWithFrame:(CGRect)frame {
    self = [super initWithFrame:frame];
    if (self) {
        _imageView = [[UIImageView alloc] initWithFrame:self.contentView.bounds];
        _imageView.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
        _imageView.contentMode = UIViewContentModeScaleAspectFill;
        _imageView.clipsToBounds = YES;
        _imageView.backgroundColor = [UIColor colorWithWhite:0.15 alpha:1];
        [self.contentView addSubview:_imageView];
    }
    return self;
}

- (void)prepareForReuse {
    [super prepareForReuse];
    self.imageView.image = nil;
    self.file = @"";
}

@end

#pragma mark - Viewer

@interface FootosPhotoViewer () <UIPageViewControllerDataSource,
                                 UIPageViewControllerDelegate,
                                 UICollectionViewDataSource,
                                 UICollectionViewDelegateFlowLayout,
                                 UIGestureRecognizerDelegate>
@property (nonatomic, copy) NSArray<NSDictionary *> *items;
@property (nonatomic, assign) NSInteger currentIndex;
@property (nonatomic, copy, nullable) void (^onDismiss)(NSInteger);
@property (nonatomic, strong) UIView *backdrop;
@property (nonatomic, strong) UIPageViewController *pager;
@property (nonatomic, strong) UIView *topBar;
@property (nonatomic, strong) UILabel *titleLabel;
@property (nonatomic, strong) UILabel *subtitleLabel;
@property (nonatomic, strong) UIButton *closeButton;
@property (nonatomic, strong) UIView *bottomBar;
@property (nonatomic, strong) UICollectionView *strip;
@property (nonatomic, assign) BOOL chromeVisible;
@property (nonatomic, assign) BOOL stripDrivenByUser;
@property (nonatomic, strong) UIPanGestureRecognizer *dismissPan;
@property (nonatomic, strong) NSDateFormatter *parseFormatter;
@end

static __weak FootosPhotoViewer *sCurrentViewer;

@implementation FootosPhotoViewer

+ (FootosPhotoViewer *)current {
    return sCurrentViewer;
}

+ (void)presentFrom:(UIViewController *)presenter
              items:(NSArray<NSDictionary *> *)items
              index:(NSInteger)index
          onDismiss:(void (^)(NSInteger))onDismiss {
    if (items.count == 0 || sCurrentViewer) {
        return;
    }
    FootosPhotoViewer *viewer = [[FootosPhotoViewer alloc] init];
    viewer.items = items;
    viewer.currentIndex = MAX(0, MIN((NSInteger)items.count - 1, index));
    viewer.onDismiss = onDismiss;
    viewer.modalPresentationStyle = UIModalPresentationOverFullScreen;
    viewer.modalTransitionStyle = UIModalTransitionStyleCrossDissolve;
    viewer.modalPresentationCapturesStatusBarAppearance = YES;
    sCurrentViewer = viewer;
    [presenter presentViewController:viewer animated:YES completion:nil];
}

- (UIStatusBarStyle)preferredStatusBarStyle {
    return UIStatusBarStyleLightContent;
}

- (void)viewDidLoad {
    [super viewDidLoad];
    self.view.backgroundColor = UIColor.clearColor;
    self.chromeVisible = YES;

    self.parseFormatter = [[NSDateFormatter alloc] init];
    self.parseFormatter.locale = [NSLocale localeWithLocaleIdentifier:@"en_US_POSIX"];
    self.parseFormatter.dateFormat = @"yyyy-MM-dd HH:mm:ss";

    self.backdrop = [[UIView alloc] initWithFrame:self.view.bounds];
    self.backdrop.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
    self.backdrop.backgroundColor = UIColor.blackColor;
    [self.view addSubview:self.backdrop];

    self.pager = [[UIPageViewController alloc]
        initWithTransitionStyle:UIPageViewControllerTransitionStyleScroll
          navigationOrientation:UIPageViewControllerNavigationOrientationHorizontal
                        options:@{UIPageViewControllerOptionInterPageSpacingKey: @(kPageSpacing)}];
    self.pager.dataSource = self;
    self.pager.delegate = self;
    [self addChildViewController:self.pager];
    self.pager.view.frame = self.view.bounds;
    self.pager.view.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
    self.pager.view.backgroundColor = UIColor.clearColor;
    [self.view addSubview:self.pager.view];
    [self.pager didMoveToParentViewController:self];
    [self.pager setViewControllers:@[[self pageAtIndex:self.currentIndex]]
                         direction:UIPageViewControllerNavigationDirectionForward
                          animated:NO
                        completion:nil];

    [self buildChrome];
    [self buildGestures];
    [self updateTitle];
}

- (void)viewDidAppear:(BOOL)animated {
    [super viewDidAppear:animated];
    [self centerStripOnIndex:self.currentIndex animated:NO];
}

#pragma mark Chrome

- (void)buildChrome {
    self.topBar = [[UIView alloc] init];
    self.topBar.backgroundColor = [UIColor colorWithWhite:0 alpha:0.35];
    [self.view addSubview:self.topBar];

    self.titleLabel = [[UILabel alloc] init];
    self.titleLabel.font = [UIFont systemFontOfSize:16 weight:UIFontWeightSemibold];
    self.titleLabel.textColor = UIColor.whiteColor;
    self.titleLabel.textAlignment = NSTextAlignmentCenter;
    [self.topBar addSubview:self.titleLabel];

    self.subtitleLabel = [[UILabel alloc] init];
    self.subtitleLabel.font = [UIFont systemFontOfSize:12 weight:UIFontWeightRegular];
    self.subtitleLabel.textColor = [UIColor colorWithWhite:1 alpha:0.75];
    self.subtitleLabel.textAlignment = NSTextAlignmentCenter;
    [self.topBar addSubview:self.subtitleLabel];

    self.closeButton = [UIButton buttonWithType:UIButtonTypeSystem];
    UIImageSymbolConfiguration *config = [UIImageSymbolConfiguration configurationWithPointSize:18 weight:UIImageSymbolWeightSemibold];
    [self.closeButton setImage:[UIImage systemImageNamed:@"chevron.left" withConfiguration:config] forState:UIControlStateNormal];
    self.closeButton.tintColor = UIColor.whiteColor;
    self.closeButton.accessibilityLabel = @"Back";
    [self.closeButton addTarget:self action:@selector(closeTapped) forControlEvents:UIControlEventTouchUpInside];
    [self.topBar addSubview:self.closeButton];

    self.bottomBar = [[UIView alloc] init];
    self.bottomBar.backgroundColor = [UIColor colorWithWhite:0 alpha:0.35];
    [self.view addSubview:self.bottomBar];

    UICollectionViewFlowLayout *layout = [[UICollectionViewFlowLayout alloc] init];
    layout.scrollDirection = UICollectionViewScrollDirectionHorizontal;
    layout.itemSize = CGSizeMake(kThumbW, kThumbH);
    layout.minimumLineSpacing = kThumbGap;
    layout.minimumInteritemSpacing = 0;
    self.strip = [[UICollectionView alloc] initWithFrame:CGRectZero collectionViewLayout:layout];
    self.strip.backgroundColor = UIColor.clearColor;
    self.strip.showsHorizontalScrollIndicator = NO;
    self.strip.decelerationRate = UIScrollViewDecelerationRateFast;
    self.strip.dataSource = self;
    self.strip.delegate = self;
    self.strip.hidden = self.items.count < 2;
    [self.strip registerClass:[FootosThumbCell class] forCellWithReuseIdentifier:@"thumb"];
    [self.bottomBar addSubview:self.strip];
    self.bottomBar.hidden = self.items.count < 2;
}

- (void)viewDidLayoutSubviews {
    [super viewDidLayoutSubviews];
    CGRect bounds = self.view.bounds;
    UIEdgeInsets safe = self.view.safeAreaInsets;

    CGFloat topHeight = safe.top + 48;
    self.topBar.frame = CGRectMake(0, 0, bounds.size.width, topHeight);
    self.closeButton.frame = CGRectMake(safe.left + 4, safe.top + 2, 44, 44);
    self.titleLabel.frame = CGRectMake(56, safe.top + 5, bounds.size.width - 112, 20);
    self.subtitleLabel.frame = CGRectMake(56, safe.top + 25, bounds.size.width - 112, 16);

    CGFloat bottomHeight = kStripHeight + safe.bottom;
    self.bottomBar.frame = CGRectMake(0, bounds.size.height - bottomHeight, bounds.size.width, bottomHeight);
    CGFloat inset = (bounds.size.width - kThumbW) / 2;
    BOOL insetChanged = fabs(self.strip.contentInset.left - inset) > 0.5;
    self.strip.frame = CGRectMake(0, (kStripHeight - kThumbH) / 2, bounds.size.width, kThumbH);
    self.strip.contentInset = UIEdgeInsetsMake(0, inset, 0, inset);
    if (insetChanged && !self.stripDrivenByUser) {
        [self centerStripOnIndex:self.currentIndex animated:NO];
    }
}

- (void)setChromeVisible:(BOOL)visible animated:(BOOL)animated {
    self.chromeVisible = visible;
    void (^apply)(void) = ^{
        self.topBar.alpha = visible ? 1 : 0;
        self.bottomBar.alpha = visible ? 1 : 0;
    };
    if (animated) {
        [UIView animateWithDuration:0.2 animations:apply];
    } else {
        apply();
    }
}

- (void)updateTitle {
    NSString *raw = FVString(self.items[self.currentIndex], @"date_taken");
    raw = [[raw stringByReplacingOccurrencesOfString:@"\"" withString:@""]
        stringByReplacingOccurrencesOfString:@"T" withString:@" "];
    if (raw.length >= 10 && [raw characterAtIndex:4] == ':') {
        // EXIF style "yyyy:MM:dd"
        raw = [raw stringByReplacingCharactersInRange:NSMakeRange(4, 1) withString:@"-"];
        raw = [raw stringByReplacingCharactersInRange:NSMakeRange(7, 1) withString:@"-"];
    }
    if (raw.length > 19) {
        raw = [raw substringToIndex:19];
    }
    NSDate *date = raw.length == 19 ? [self.parseFormatter dateFromString:raw] : nil;
    if (!date) {
        self.titleLabel.text = @"";
        self.subtitleLabel.text = @"";
        return;
    }
    NSCalendar *calendar = NSCalendar.currentCalendar;
    BOOL sameYear = [calendar component:NSCalendarUnitYear fromDate:date]
        == [calendar component:NSCalendarUnitYear fromDate:[NSDate date]];
    NSDateFormatter *title = [[NSDateFormatter alloc] init];
    [title setLocalizedDateFormatFromTemplate:sameYear ? @"MMMMd" : @"yMMMMd"];
    NSDateFormatter *time = [[NSDateFormatter alloc] init];
    time.dateStyle = NSDateFormatterNoStyle;
    time.timeStyle = NSDateFormatterShortStyle;
    self.titleLabel.text = [title stringFromDate:date];
    self.subtitleLabel.text = [time stringFromDate:date];
}

#pragma mark Paging

- (FootosPhotoPage *)pageAtIndex:(NSInteger)index {
    if (index < 0 || index >= (NSInteger)self.items.count) {
        return nil;
    }
    FootosPhotoPage *page = [[FootosPhotoPage alloc] init];
    page.item = self.items[index];
    page.index = index;
    return page;
}

- (FootosPhotoPage *)currentPage {
    return (FootosPhotoPage *)self.pager.viewControllers.firstObject;
}

- (UIViewController *)pageViewController:(UIPageViewController *)pageViewController
      viewControllerBeforeViewController:(UIViewController *)viewController {
    return [self pageAtIndex:((FootosPhotoPage *)viewController).index - 1];
}

- (UIViewController *)pageViewController:(UIPageViewController *)pageViewController
       viewControllerAfterViewController:(UIViewController *)viewController {
    return [self pageAtIndex:((FootosPhotoPage *)viewController).index + 1];
}

- (void)pageViewController:(UIPageViewController *)pageViewController
        didFinishAnimating:(BOOL)finished
   previousViewControllers:(NSArray<UIViewController *> *)previousViewControllers
       transitionCompleted:(BOOL)completed {
    if (!completed) {
        return;
    }
    self.currentIndex = [self currentPage].index;
    [self updateTitle];
    [self centerStripOnIndex:self.currentIndex animated:YES];
}

/// Jump straight to a page (used by the scrubber).
- (void)showIndex:(NSInteger)index {
    if (index == self.currentIndex || index < 0 || index >= (NSInteger)self.items.count) {
        return;
    }
    self.currentIndex = index;
    [self.pager setViewControllers:@[[self pageAtIndex:index]]
                         direction:UIPageViewControllerNavigationDirectionForward
                          animated:NO
                        completion:nil];
    [self updateTitle];
}

#pragma mark Scrubber

- (CGFloat)stripOffsetForIndex:(NSInteger)index {
    return index * (kThumbW + kThumbGap) - self.strip.contentInset.left;
}

- (NSInteger)stripIndexForOffset:(CGFloat)offsetX {
    NSInteger index = (NSInteger)llround((offsetX + self.strip.contentInset.left) / (kThumbW + kThumbGap));
    return MAX(0, MIN((NSInteger)self.items.count - 1, index));
}

- (void)centerStripOnIndex:(NSInteger)index animated:(BOOL)animated {
    if (self.strip.hidden) {
        return;
    }
    [self.strip setContentOffset:CGPointMake([self stripOffsetForIndex:index], 0) animated:animated];
}

- (NSInteger)collectionView:(UICollectionView *)collectionView numberOfItemsInSection:(NSInteger)section {
    return self.items.count;
}

- (UICollectionViewCell *)collectionView:(UICollectionView *)collectionView
                  cellForItemAtIndexPath:(NSIndexPath *)indexPath {
    FootosThumbCell *cell = [collectionView dequeueReusableCellWithReuseIdentifier:@"thumb" forIndexPath:indexPath];
    NSString *file = FVString(self.items[indexPath.item], @"thumb");
    cell.file = file;
    __weak FootosThumbCell *weakCell = cell;
    FVLoadThumb(file, ^(UIImage *image) {
        if ([weakCell.file isEqualToString:file]) {
            weakCell.imageView.image = image;
        }
    });
    return cell;
}

- (void)collectionView:(UICollectionView *)collectionView didSelectItemAtIndexPath:(NSIndexPath *)indexPath {
    [self showIndex:indexPath.item];
    [self centerStripOnIndex:indexPath.item animated:YES];
}

- (void)scrollViewWillBeginDragging:(UIScrollView *)scrollView {
    if (scrollView == self.strip) {
        self.stripDrivenByUser = YES;
    }
}

// While the finger (or its momentum) moves the strip, the centred thumbnail is the current photo.
- (void)scrollViewDidScroll:(UIScrollView *)scrollView {
    if (scrollView != self.strip || !self.stripDrivenByUser) {
        return;
    }
    [self showIndex:[self stripIndexForOffset:scrollView.contentOffset.x]];
}

// Always come to rest with a thumbnail exactly centred.
- (void)scrollViewWillEndDragging:(UIScrollView *)scrollView
                     withVelocity:(CGPoint)velocity
              targetContentOffset:(inout CGPoint *)targetContentOffset {
    if (scrollView != self.strip) {
        return;
    }
    NSInteger index = [self stripIndexForOffset:targetContentOffset->x];
    targetContentOffset->x = [self stripOffsetForIndex:index];
}

- (void)scrollViewDidEndDragging:(UIScrollView *)scrollView willDecelerate:(BOOL)decelerate {
    if (scrollView == self.strip && !decelerate) {
        self.stripDrivenByUser = NO;
    }
}

- (void)scrollViewDidEndDecelerating:(UIScrollView *)scrollView {
    if (scrollView == self.strip) {
        self.stripDrivenByUser = NO;
    }
}

#pragma mark Gestures

- (void)buildGestures {
    UITapGestureRecognizer *doubleTap = [[UITapGestureRecognizer alloc] initWithTarget:self action:@selector(handleDoubleTap:)];
    doubleTap.numberOfTapsRequired = 2;
    doubleTap.delegate = self;
    [self.pager.view addGestureRecognizer:doubleTap];

    UITapGestureRecognizer *singleTap = [[UITapGestureRecognizer alloc] initWithTarget:self action:@selector(handleSingleTap:)];
    [singleTap requireGestureRecognizerToFail:doubleTap];
    singleTap.delegate = self;
    [self.pager.view addGestureRecognizer:singleTap];

    UILongPressGestureRecognizer *longPress = [[UILongPressGestureRecognizer alloc] initWithTarget:self action:@selector(handleLongPress:)];
    longPress.minimumPressDuration = 0.25;
    longPress.delegate = self;
    [self.pager.view addGestureRecognizer:longPress];

    self.dismissPan = [[UIPanGestureRecognizer alloc] initWithTarget:self action:@selector(handleDismissPan:)];
    self.dismissPan.delegate = self;
    [self.pager.view addGestureRecognizer:self.dismissPan];
}

- (void)handleSingleTap:(UITapGestureRecognizer *)tap {
    FootosPhotoPage *page = [self currentPage];
    if ([page isVideo]) {
        BOOL playing = [page togglePlayback];
        [self setChromeVisible:!playing animated:YES];
        return;
    }
    [self setChromeVisible:!self.chromeVisible animated:YES];
}

- (void)handleDoubleTap:(UITapGestureRecognizer *)tap {
    FootosPhotoPage *page = [self currentPage];
    [page toggleZoomAtPoint:[tap locationInView:page.view]];
}

- (void)handleLongPress:(UILongPressGestureRecognizer *)press {
    FootosPhotoPage *page = [self currentPage];
    if (press.state == UIGestureRecognizerStateBegan) {
        [page startLivePhoto];
    } else if (press.state == UIGestureRecognizerStateEnded
               || press.state == UIGestureRecognizerStateCancelled
               || press.state == UIGestureRecognizerStateFailed) {
        [page stopLivePhoto];
    }
}

// Swipe down to dismiss only starts on a mostly-vertical downward drag of an un-zoomed photo.
- (BOOL)gestureRecognizerShouldBegin:(UIGestureRecognizer *)gestureRecognizer {
    if (gestureRecognizer != self.dismissPan) {
        return YES;
    }
    if ([[self currentPage] isZoomed]) {
        return NO;
    }
    CGPoint velocity = [self.dismissPan velocityInView:self.view];
    return velocity.y > 0 && fabs(velocity.y) > fabs(velocity.x) * 1.2;
}

// The page's own scroll views also track pans; let the dismiss pan run alongside
// them instead of losing the race, and freeze horizontal paging while it is active.
- (BOOL)gestureRecognizer:(UIGestureRecognizer *)gestureRecognizer
    shouldRecognizeSimultaneouslyWithGestureRecognizer:(UIGestureRecognizer *)other {
    return gestureRecognizer == self.dismissPan && [other isKindOfClass:[UIPanGestureRecognizer class]];
}

- (void)setPagingEnabled:(BOOL)enabled {
    for (UIView *view in self.pager.view.subviews) {
        if ([view isKindOfClass:[UIScrollView class]]) {
            ((UIScrollView *)view).scrollEnabled = enabled;
        }
    }
}

- (void)handleDismissPan:(UIPanGestureRecognizer *)pan {
    CGPoint translation = [pan translationInView:self.view];
    CGFloat progress = MIN(1.0, MAX(0.0, translation.y) / 320.0);

    switch (pan.state) {
        case UIGestureRecognizerStateBegan:
            [self setPagingEnabled:NO];
            // fall through
        case UIGestureRecognizerStateChanged: {
            CGFloat scale = 1.0 - progress * 0.3;
            CGAffineTransform transform = CGAffineTransformMakeTranslation(translation.x * 0.7, MAX(0.0, translation.y));
            self.pager.view.transform = CGAffineTransformScale(transform, scale, scale);
            self.backdrop.alpha = 1.0 - progress;
            CGFloat chromeAlpha = self.chromeVisible ? MAX(0.0, 1.0 - progress * 4.0) : 0.0;
            self.topBar.alpha = chromeAlpha;
            self.bottomBar.alpha = chromeAlpha;
            break;
        }
        case UIGestureRecognizerStateEnded:
        case UIGestureRecognizerStateCancelled:
        case UIGestureRecognizerStateFailed: {
            [self setPagingEnabled:YES];
            CGFloat velocityY = [pan velocityInView:self.view].y;
            BOOL dismiss = pan.state == UIGestureRecognizerStateEnded
                && (translation.y > kDismissDistance || velocityY > kDismissVelocity);
            if (dismiss) {
                [self dismissAnimatedFromDrag];
            } else {
                [UIView animateWithDuration:0.32
                                      delay:0
                     usingSpringWithDamping:0.82
                      initialSpringVelocity:0
                                    options:UIViewAnimationOptionAllowUserInteraction
                                 animations:^{
                    self.pager.view.transform = CGAffineTransformIdentity;
                    self.backdrop.alpha = 1.0;
                    self.topBar.alpha = self.chromeVisible ? 1 : 0;
                    self.bottomBar.alpha = self.chromeVisible ? 1 : 0;
                } completion:nil];
            }
            break;
        }
        default:
            break;
    }
}

#pragma mark Dismiss

- (void)finishDismiss {
    NSInteger index = self.currentIndex;
    void (^callback)(NSInteger) = self.onDismiss;
    self.onDismiss = nil;
    sCurrentViewer = nil;
    [self dismissViewControllerAnimated:NO completion:^{
        // Hand audio back (resumes the user's music) once our players have stopped.
        AVAudioSession *session = [AVAudioSession sharedInstance];
        [session setActive:NO
               withOptions:AVAudioSessionSetActiveOptionNotifyOthersOnDeactivation
                     error:nil];
        [session setCategory:AVAudioSessionCategoryAmbient error:nil];
        if (callback) {
            callback(index);
        }
    }];
}

- (void)dismissAnimatedFromDrag {
    [[self currentPage] stopPlayback];
    [UIView animateWithDuration:0.22 animations:^{
        CGAffineTransform t = self.pager.view.transform;
        self.pager.view.transform = CGAffineTransformScale(CGAffineTransformTranslate(t, 0, 160), 0.7, 0.7);
        self.pager.view.alpha = 0;
        self.backdrop.alpha = 0;
        self.topBar.alpha = 0;
        self.bottomBar.alpha = 0;
    } completion:^(BOOL finished) {
        [self finishDismiss];
    }];
}

- (void)closeTapped {
    [[self currentPage] stopPlayback];
    [UIView animateWithDuration:0.2 animations:^{
        self.view.alpha = 0;
    } completion:^(BOOL finished) {
        [self finishDismiss];
    }];
}

@end
