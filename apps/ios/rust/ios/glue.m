// glue.m — нативная iOS-часть burmalda57. Компилируется build.rs (крейт `cc`, ARC).
// Это аналог Kotlin-классов Android-версии:
//   EsiaAuthActivity       → BWLoginVC (WKWebView)
//   KeystoreCrypto         → bw_keychain_get / bw_keychain_set
//   AvatarPickerActivity   → bw_pick_avatar (PHPicker)
//   Notifier / GradesWorker→ bw_notify_* / bw_bg_* (UserNotifications / BackgroundTasks)
//
// Функции bw_on_* реализованы на стороне Rust (src/ios.rs).

#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>
#import <Security/Security.h>
#import <UserNotifications/UserNotifications.h>
#import <BackgroundTasks/BackgroundTasks.h>
#import <PhotosUI/PhotosUI.h>
#include <stdint.h>
#include <string.h>

// ---- Rust → вызываются отсюда ----
extern void bw_on_esia_token(const char *token);
extern void bw_on_avatar_bytes(const uint8_t *data, size_t len);
extern int bw_poll_blocking(void);

static NSString *const kLoginURL = @"https://passport.obr57.ru/auth/esia/redirect/?returnTo=https://one.obr57.ru";
static NSString *const kKeychainService = @"ru.burmalda.journal.kek";
static NSString *const kBGTaskId = @"ru.burmalda.journal.refresh";

// ===========================================================================
//  Хелперы
// ===========================================================================

static void bw_main(void (^block)(void)) {
    if ([NSThread isMainThread]) {
        block();
    } else {
        dispatch_async(dispatch_get_main_queue(), block);
    }
}

static UIWindow *bw_key_window(void) {
    UIApplication *app = UIApplication.sharedApplication;
    // Современный путь: сцены (Slint на iOS работает через UIScene).
    for (UIScene *scene in app.connectedScenes) {
        if (![scene isKindOfClass:UIWindowScene.class]) continue;
        UIWindowScene *ws = (UIWindowScene *)scene;
        if (ws.activationState == UISceneActivationStateBackground ||
            ws.activationState == UISceneActivationStateUnattached) continue;
        for (UIWindow *w in ws.windows) {
            if (w.isKeyWindow) return w;
        }
        if (ws.windows.firstObject) return ws.windows.firstObject;
    }
    // Запасной путь для окружений без сцен.
    for (UIWindow *w in app.windows) {
        if (w.isKeyWindow) return w;
    }
    return app.windows.firstObject;
}

static UIViewController *bw_top_vc(void) {
    UIViewController *vc = bw_key_window().rootViewController;
    while (vc.presentedViewController) vc = vc.presentedViewController;
    return vc;
}

// ===========================================================================
//  Каталог хранилища
// ===========================================================================

size_t bw_app_support_dir(char *buf, size_t cap) {
    NSFileManager *fm = NSFileManager.defaultManager;
    NSURL *base = [[fm URLsForDirectory:NSApplicationSupportDirectory inDomains:NSUserDomainMask] firstObject];
    if (!base) return 0;
    [fm createDirectoryAtURL:base withIntermediateDirectories:YES attributes:nil error:nil];
    // Android-версия отключает бэкап (allowBackup=false) — здесь то же самое.
    [base setResourceValue:@YES forKey:NSURLIsExcludedFromBackupKey error:nil];
    const char *p = base.path.fileSystemRepresentation;
    if (!p) return 0;
    size_t n = strlen(p);
    if (n + 1 > cap) return 0;
    memcpy(buf, p, n + 1);
    return n;
}

// ===========================================================================
//  Вход через Госуслуги (ЕСИА) — WKWebView
// ===========================================================================

@interface BWLoginVC : UIViewController
@property (nonatomic, strong) WKWebView *web;
@property (nonatomic, strong) NSTimer *timer;
@property (nonatomic, assign) BOOL sent;
@end

@implementation BWLoginVC

- (void)viewDidLoad {
    [super viewDidLoad];
    self.view.backgroundColor = UIColor.systemBackgroundColor;
    self.title = @"Вход через Госуслуги";
    self.navigationItem.leftBarButtonItem =
        [[UIBarButtonItem alloc] initWithBarButtonSystemItem:UIBarButtonSystemItemCancel
                                                      target:self
                                                      action:@selector(cancelTapped)];

    WKWebViewConfiguration *cfg = [WKWebViewConfiguration new];
    // Непостоянное хранилище: куки живут только в памяти и исчезают вместе с экраном.
    // Это аналог removeAllCookies() в Android-версии — сессия ЕСИА не переживёт «Выйти».
    cfg.websiteDataStore = [WKWebsiteDataStore nonPersistentDataStore];
    self.web = [[WKWebView alloc] initWithFrame:self.view.bounds configuration:cfg];
    self.web.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
    [self.view addSubview:self.web];
    [self.web loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:kLoginURL]]];

    // Куки приходят асинхронно и без гарантированного события навигации (SPA-редиректы),
    // поэтому просто опрашиваем хранилище раз в секунду, пока экран открыт.
    __weak typeof(self) weakSelf = self;
    self.timer = [NSTimer scheduledTimerWithTimeInterval:1.0 repeats:YES block:^(NSTimer *t) {
        [weakSelf checkCookies];
    }];
}

- (void)checkCookies {
    if (self.sent) return;
    [self.web.configuration.websiteDataStore.httpCookieStore getAllCookies:^(NSArray<NSHTTPCookie *> *cookies) {
        if (self.sent) return;
        for (NSHTTPCookie *c in cookies) {
            if (![c.name isEqualToString:@"X1_SSO"]) continue;
            NSString *domain = c.domain.lowercaseString;
            if (!([domain isEqualToString:@"obr57.ru"] || [domain hasSuffix:@".obr57.ru"] ||
                  [domain isEqualToString:@"shkolove.ru"] || [domain hasSuffix:@".shkolove.ru"])) continue;
            NSString *v = [c.value stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
            if (v.length == 0 || [v isEqualToString:@"deleted"]) continue;

            self.sent = YES;
            [self.timer invalidate];
            self.timer = nil;
            bw_on_esia_token(v.UTF8String);
            [self closeScreen];
            return;
        }
    }];
}

- (void)closeScreen {
    UIViewController *target = self;
    if (self.navigationController) target = self.navigationController;
    [target dismissViewControllerAnimated:YES completion:nil];
}

- (void)cancelTapped {
    [self.timer invalidate];
    self.timer = nil;
    [self closeScreen];
}

- (void)dealloc {
    [self.timer invalidate];
}

@end

void bw_launch_login(void) {
    bw_main(^{
        UIViewController *top = bw_top_vc();
        if (!top) return;
        BWLoginVC *vc = [BWLoginVC new];
        UINavigationController *nav = [[UINavigationController alloc] initWithRootViewController:vc];
        nav.modalPresentationStyle = UIModalPresentationFullScreen;
        [top presentViewController:nav animated:YES completion:nil];
    });
}

// ===========================================================================
//  Toast, внешние ссылки, тема, безопасные области
// ===========================================================================

void bw_show_toast(const char *text) {
    NSString *s = [NSString stringWithUTF8String:(text ? text : "")];
    bw_main(^{
        UIWindow *w = bw_key_window();
        if (!w) return;
        UILabel *l = [UILabel new];
        l.text = s;
        l.numberOfLines = 0;
        l.textAlignment = NSTextAlignmentCenter;
        l.textColor = UIColor.whiteColor;
        l.font = [UIFont systemFontOfSize:14];
        l.backgroundColor = [UIColor colorWithWhite:0.1 alpha:0.92];
        l.layer.cornerRadius = 14;
        l.layer.masksToBounds = YES;

        CGFloat maxW = w.bounds.size.width - 64;
        CGSize sz = [l sizeThatFits:CGSizeMake(maxW - 32, CGFLOAT_MAX)];
        CGFloat bw = MIN(maxW, sz.width + 32);
        CGFloat bh = sz.height + 20;
        CGFloat y = w.bounds.size.height - w.safeAreaInsets.bottom - 110 - bh;
        l.frame = CGRectMake((w.bounds.size.width - bw) / 2, y, bw, bh);
        l.alpha = 0;
        [w addSubview:l];
        [UIView animateWithDuration:0.2 animations:^{ l.alpha = 1; } completion:^(BOOL f1) {
            [UIView animateWithDuration:0.3 delay:2.0 options:0 animations:^{ l.alpha = 0; }
                             completion:^(BOOL f2) { [l removeFromSuperview]; }];
        }];
    });
}

void bw_open_url(const char *url) {
    NSString *s = [NSString stringWithUTF8String:(url ? url : "")];
    bw_main(^{
        NSURL *u = [NSURL URLWithString:s];
        if (u) [UIApplication.sharedApplication openURL:u options:@{} completionHandler:nil];
    });
}

int bw_is_dark(void) {
    __block int dark = 0;
    void (^read)(void) = ^{
        UIWindow *w = bw_key_window();
        UITraitCollection *tc = w ? w.traitCollection : UIScreen.mainScreen.traitCollection;
        dark = (tc.userInterfaceStyle == UIUserInterfaceStyleDark) ? 1 : 0;
    };
    if ([NSThread isMainThread]) read(); else dispatch_sync(dispatch_get_main_queue(), read);
    return dark;
}

void bw_safe_area(float *top, float *bottom) {
    __block float t = 0, b = 0;
    void (^read)(void) = ^{
        UIWindow *w = bw_key_window();
        if (w) {
            t = (float)w.safeAreaInsets.top;
            b = (float)w.safeAreaInsets.bottom;
        }
    };
    if ([NSThread isMainThread]) read(); else dispatch_sync(dispatch_get_main_queue(), read);
    if (top) *top = t;
    if (bottom) *bottom = b;
}

// ===========================================================================
//  Keychain: 0 — ок, 1 — записи нет, -1 — ошибка (в т.ч. «устройство заблокировано»)
// ===========================================================================

static NSMutableDictionary *bw_kc_query(const char *account) {
    return [@{
        (__bridge id)kSecClass: (__bridge id)kSecClassGenericPassword,
        (__bridge id)kSecAttrService: kKeychainService,
        (__bridge id)kSecAttrAccount: [NSString stringWithUTF8String:account],
    } mutableCopy];
}

int bw_keychain_get(const char *account, uint8_t *out, size_t cap, size_t *out_len) {
    if (!account || !out || !out_len) return -1;
    NSMutableDictionary *q = bw_kc_query(account);
    q[(__bridge id)kSecReturnData] = @YES;
    q[(__bridge id)kSecMatchLimit] = (__bridge id)kSecMatchLimitOne;
    CFTypeRef res = NULL;
    OSStatus st = SecItemCopyMatching((__bridge CFDictionaryRef)q, &res);
    if (st == errSecItemNotFound) return 1;
    if (st != errSecSuccess || !res) return -1;
    NSData *d = (__bridge_transfer NSData *)res;
    if (d.length > cap) return -1;
    memcpy(out, d.bytes, d.length);
    *out_len = d.length;
    return 0;
}

int bw_keychain_set(const char *account, const uint8_t *data, size_t len) {
    if (!account || !data) return -1;
    NSData *blob = [NSData dataWithBytes:data length:len];
    NSMutableDictionary *q = bw_kc_query(account);

    // Сначала пробуем обновить существующую запись.
    OSStatus st = SecItemUpdate((__bridge CFDictionaryRef)q,
                                (__bridge CFDictionaryRef)@{ (__bridge id)kSecValueData: blob });
    if (st == errSecSuccess) return 0;
    if (st != errSecItemNotFound) return -1;

    // AfterFirstUnlock — чтобы фоновый опрос работал при заблокированном экране;
    // ThisDeviceOnly — ключ не уезжает в бэкапы и на другие устройства.
    q[(__bridge id)kSecValueData] = blob;
    q[(__bridge id)kSecAttrAccessible] = (__bridge id)kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly;
    st = SecItemAdd((__bridge CFDictionaryRef)q, NULL);
    return st == errSecSuccess ? 0 : -1;
}

// ===========================================================================
//  Аватар: PHPicker → ужатие до 512 px → JPEG → Rust
// ===========================================================================

static NSData *bw_downscaled_jpeg(UIImage *img, CGFloat maxSide) {
    CGSize s = img.size;
    CGFloat m = MAX(s.width, s.height);
    if (m <= 0) return nil;
    CGFloat k = MIN(1.0, maxSide / m);
    CGSize ns = CGSizeMake(MAX(1, floor(s.width * k)), MAX(1, floor(s.height * k)));
    UIGraphicsImageRendererFormat *fmt = [UIGraphicsImageRendererFormat preferredFormat];
    fmt.scale = 1.0;
    fmt.opaque = YES;
    UIGraphicsImageRenderer *r = [[UIGraphicsImageRenderer alloc] initWithSize:ns format:fmt];
    UIImage *out = [r imageWithActions:^(UIGraphicsImageRendererContext *ctx) {
        // drawInRect учитывает EXIF-ориентацию UIImage — повёрнутые фото с камеры выйдут ровными.
        [img drawInRect:CGRectMake(0, 0, ns.width, ns.height)];
    }];
    return UIImageJPEGRepresentation(out, 0.9);
}

@interface BWPickerDelegate : NSObject <PHPickerViewControllerDelegate>
@end

@implementation BWPickerDelegate
- (void)picker:(PHPickerViewController *)picker didFinishPicking:(NSArray<PHPickerResult *> *)results {
    [picker dismissViewControllerAnimated:YES completion:nil];
    PHPickerResult *r = results.firstObject;
    if (!r || ![r.itemProvider canLoadObjectOfClass:UIImage.class]) return;
    [r.itemProvider loadObjectOfClass:UIImage.class
                    completionHandler:^(__kindof id<NSItemProviderReading> obj, NSError *err) {
        if (![obj isKindOfClass:UIImage.class]) return;
        NSData *jpg = bw_downscaled_jpeg((UIImage *)obj, 512);
        if (jpg.length > 0) bw_on_avatar_bytes(jpg.bytes, jpg.length);
    }];
}
@end

static BWPickerDelegate *gPickerDelegate; // delegate у PHPicker слабый — держим сами

void bw_pick_avatar(void) {
    bw_main(^{
        UIViewController *top = bw_top_vc();
        if (!top) return;
        PHPickerConfiguration *cfg = [[PHPickerConfiguration alloc] init];
        cfg.filter = [PHPickerFilter imagesFilter];
        cfg.selectionLimit = 1;
        PHPickerViewController *p = [[PHPickerViewController alloc] initWithConfiguration:cfg];
        gPickerDelegate = [BWPickerDelegate new];
        p.delegate = gPickerDelegate;
        [top presentViewController:p animated:YES completion:nil];
    });
}

// ===========================================================================
//  Уведомления (локальные: Apple Push требует платного аккаунта и сервера)
// ===========================================================================

@interface BWNotifDelegate : NSObject <UNUserNotificationCenterDelegate>
@end

@implementation BWNotifDelegate
// Без этого iOS не показывает баннер, пока приложение на экране.
- (void)userNotificationCenter:(UNUserNotificationCenter *)center
       willPresentNotification:(UNNotification *)notification
         withCompletionHandler:(void (^)(UNNotificationPresentationOptions))handler {
    handler(UNNotificationPresentationOptionBanner | UNNotificationPresentationOptionList |
            UNNotificationPresentationOptionSound);
}
@end

static BWNotifDelegate *gNotifDelegate;

void bw_notify_request_permission(void) {
    UNUserNotificationCenter *c = UNUserNotificationCenter.currentNotificationCenter;
    if (!gNotifDelegate) gNotifDelegate = [BWNotifDelegate new];
    c.delegate = gNotifDelegate;
    [c requestAuthorizationWithOptions:(UNAuthorizationOptionAlert | UNAuthorizationOptionSound | UNAuthorizationOptionBadge)
                     completionHandler:^(BOOL granted, NSError *error) {}];
}

void bw_notify_show(int nid, const char *title, const char *text) {
    UNMutableNotificationContent *c = [UNMutableNotificationContent new];
    c.title = [NSString stringWithUTF8String:(title ? title : "")];
    c.body = [NSString stringWithUTF8String:(text ? text : "")];
    c.sound = UNNotificationSound.defaultSound;
    UNNotificationRequest *r =
        [UNNotificationRequest requestWithIdentifier:[NSString stringWithFormat:@"bw-%d", nid]
                                             content:c
                                             trigger:nil];
    [UNUserNotificationCenter.currentNotificationCenter addNotificationRequest:r withCompletionHandler:nil];
}

// ===========================================================================
//  Фоновый опрос оценок — BGAppRefreshTask
//  iOS САМА решает, когда запускать задачу (обычно раз в несколько часов, чаще —
//  для приложений, которыми часто пользуются). Гарантий «каждые 30 минут», как у
//  Android WorkManager, на iOS без Apple Push не бывает.
// ===========================================================================

void bw_bg_schedule(int minutes) {
    @try {
        BGAppRefreshTaskRequest *req = [[BGAppRefreshTaskRequest alloc] initWithIdentifier:kBGTaskId];
        req.earliestBeginDate = [NSDate dateWithTimeIntervalSinceNow:minutes * 60.0];
        NSError *err = nil;
        if (![BGTaskScheduler.sharedScheduler submitTaskRequest:req error:&err]) {
            NSLog(@"burmalda57: BGTask schedule failed: %@", err);
        }
    } @catch (NSException *e) {
        NSLog(@"burmalda57: BGTask schedule exception: %@", e);
    }
}

void bw_bg_cancel(void) {
    @try {
        [BGTaskScheduler.sharedScheduler cancelTaskRequestWithIdentifier:kBGTaskId];
    } @catch (NSException *e) {
        NSLog(@"burmalda57: BGTask cancel exception: %@", e);
    }
}

void bw_bg_register(void) {
    @try {
        [BGTaskScheduler.sharedScheduler
            registerForTaskWithIdentifier:kBGTaskId
                               usingQueue:nil
                            launchHandler:^(__kindof BGTask *task) {
            bw_bg_schedule(30); // сразу ставим следующий запуск
            __block BOOL finished = NO;
            NSObject *lock = [NSObject new];
            task.expirationHandler = ^{
                @synchronized (lock) {
                    if (finished) return;
                    finished = YES;
                }
                [task setTaskCompletedWithSuccess:NO];
            };
            dispatch_async(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
                int rc = bw_poll_blocking();
                @synchronized (lock) {
                    if (finished) return;
                    finished = YES;
                }
                [task setTaskCompletedWithSuccess:(rc == 0)];
            });
        }];
    } @catch (NSException *e) {
        NSLog(@"burmalda57: BGTask register exception: %@", e);
    }
}
