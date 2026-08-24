#import <Foundation/Foundation.h>
#import <UIKit/UIKit.h>

NS_ASSUME_NONNULL_BEGIN

/// In-process UI driver for LIGH. Starts in DEBUG / Expo dev client.
/// Connects to lighd like Metro (phone is the client). Also listens on
/// loopback for USB `iproxy` when there is no Wi-Fi.
@interface LighDevDriver : NSObject
+ (void)start;
@end

NS_ASSUME_NONNULL_END
