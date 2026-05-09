import Foundation
import OSLog
import UIKit
import UserNotifications

final class NiumaAppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    func application(
        _ application: UIApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        let token = deviceToken.map { String(format: "%02x", $0) }.joined()
        Task { @MainActor in
            PushNotificationCoordinator.shared.receiveDeviceToken(token)
        }
    }

    func application(
        _ application: UIApplication,
        didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        Logger(subsystem: "com.rainchestnut.niuma", category: "push")
            .error("apns_registration_failed error=\(error.localizedDescription, privacy: .public)")
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        let userInfo = notification.request.content.userInfo
        Task { @MainActor in
            let shouldPresent = await PushNotificationCoordinator.shared.shouldPresentNotification(userInfo)
            completionHandler(shouldPresent ? [.banner, .list, .sound] : [])
        }
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let userInfo = response.notification.request.content.userInfo
        Task { @MainActor in
            await PushNotificationCoordinator.shared.receiveNotification(userInfo)
        }
        completionHandler()
    }
}

@MainActor
final class PushNotificationCoordinator {
    static let shared = PushNotificationCoordinator()

    private weak var appModel: AppModel?
    private var pendingToken: String?
    private var pendingNotifications: [[AnyHashable: Any]] = []

    private init() {}

    func attach(_ appModel: AppModel) {
        self.appModel = appModel
        if let pendingToken {
            self.pendingToken = nil
            appModel.receivePushToken(pendingToken)
        }
        let notifications = pendingNotifications
        pendingNotifications.removeAll()
        for userInfo in notifications {
            Task { await appModel.handlePushNotification(userInfo: userInfo) }
        }
    }

    func receiveDeviceToken(_ token: String) {
        guard let appModel else {
            pendingToken = token
            return
        }
        appModel.receivePushToken(token)
    }

    func receiveNotification(_ userInfo: [AnyHashable: Any]) async {
        guard let appModel else {
            pendingNotifications.append(userInfo)
            return
        }
        await appModel.handlePushNotification(userInfo: userInfo)
    }

    func shouldPresentNotification(_ userInfo: [AnyHashable: Any]) async -> Bool {
        guard let appModel else {
            return true
        }
        return await appModel.shouldPresentPushNotification(userInfo: userInfo)
    }
}
