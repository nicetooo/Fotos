#include "bindings/bindings.h"
#import <UIKit/UIKit.h>

// Forward declaration of Swift bridge init function
extern "C" void FotosBridgeInit(void);

int main(int argc, char * argv[]) {
    // Initialize Swift photo bridge
    dispatch_async(dispatch_get_main_queue(), ^{
        FotosBridgeInit();
    });

    ffi::start_app();
    return 0;
}
