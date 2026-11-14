#import <UIKit/UIKit.h>

NS_ASSUME_NONNULL_BEGIN

/// Native full-screen photo viewer presented above the WKWebView.
///
/// Items are dictionaries with:
///   path        `ios-photo://<PHAsset localIdentifier>` or a local file path
///   thumb       local thumbnail file path (may be empty)
///   media_type  "photo" | "video" | "live_photo"
///   date_taken  "yyyy-MM-dd HH:mm:ss" / "yyyy-MM-ddTHH:mm:ss" (may be empty)
@interface FootosPhotoViewer : UIViewController

+ (void)presentFrom:(UIViewController *)presenter
              items:(NSArray<NSDictionary *> *)items
              index:(NSInteger)index
          onDismiss:(nullable void (^)(NSInteger lastIndex))onDismiss;

/// The viewer currently on screen, if any (used by tests and to avoid stacking).
+ (nullable FootosPhotoViewer *)current;

@end

NS_ASSUME_NONNULL_END
