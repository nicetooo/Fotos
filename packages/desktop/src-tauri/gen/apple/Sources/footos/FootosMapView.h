#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>

NS_ASSUME_NONNULL_BEGIN

/// The system map (MapKit), placed underneath the transparent WKWebView.
///
/// The web side keeps the buttons, the timeline and every other screen, and
/// keeps deciding what is on the map: it asks Rust for the clusters inside the
/// region this class reports, and hands the result back to be drawn. This
/// class only draws markers, moves the camera and reports region changes and
/// marker taps.
///
/// Commands (from JS, through PhotoLibraryBridge) all start with "map":
///   mapAttach      {x,y,w,h,dark}          show the map in that rect of the web view
///   mapSetFrame    {x,y,w,h}
///   mapSetFeatures {features:[…]}          markers to show; diffed by `key`
///   mapFitBounds   {w,s,e,n,padding,maxZoom,animated}
///   mapEaseTo      {lon,lat,zoom?,minZoom?,duration?}
///   mapSetTheme    {dark}
///   mapSetOverlays {enabled,rects:[[x,y,w,h],…]}   where the web UI covers the map
///   mapDetach
///
/// Events (to JS): window.__footosNativeMap.onRegion({west,south,east,north,
/// zoom,lon,lat,moving}) and .onTap({key}).
@interface FootosMapView : NSObject

+ (instancetype)shared;

/// Returns YES when `command` was a map command (handled or not).
- (BOOL)handleCommand:(NSString *)command
                 body:(NSDictionary *)body
              webView:(WKWebView *)webView
                 eval:(void (^)(NSString *js))eval;

@end

NS_ASSUME_NONNULL_END
