#import <Foundation/Foundation.h>
#include <stdlib.h>

static id<NSObject> nativeTestActivity;

static void finishNativeTestActivity(void) {
    [NSProcessInfo.processInfo endActivity:nativeTestActivity];
    nativeTestActivity = nil;
}

@interface ViemNativeTestBootstrap : NSObject
@end

@implementation ViemNativeTestBootstrap
+ (void)load {
    @autoreleasepool {
        // Tests remain user-requested work when their windows are behind
        // other applications. Keep App Nap from throttling the test host;
        // the assertion ends with this process and changes no preferences.
        nativeTestActivity = [NSProcessInfo.processInfo
            beginActivityWithOptions:NSActivityUserInitiatedAllowingIdleSystemSleep
            reason:@"Running native tests"];
        atexit(finishNativeTestActivity);
        // Native fixtures create many windows without a normal application
        // event loop. Blocked AppKit animations can exhaust dispatch workers
        // needed by file I/O. Disable automatic window transitions before any
        // fixture runs; retain real controls, drawing, and event routing.
        // NSArgumentDomain is volatile: this never changes user preferences
        // or the application, which does not link this test-support target.
        NSUserDefaults *defaults = NSUserDefaults.standardUserDefaults;
        NSMutableDictionary *arguments = [[defaults volatileDomainForName:NSArgumentDomain] mutableCopy];
        arguments[@"NSAutomaticWindowAnimationsEnabled"] = @NO;
        [defaults setVolatileDomain:arguments forName:NSArgumentDomain];
    }
}
@end
