import Foundation
import OSLog
import UIKit
import UserNotifications

extension AppModel {
    var storedPushToken: String? {
        guard let token = storage
            .string(forKey: StorageKey.apnsDeviceToken.rawValue)?
            .trimmingCharacters(in: .whitespacesAndNewlines),
              !token.isEmpty
        else {
            return nil
        }
        return token
    }

    /// Requests APNs authorization and starts device-token registration for background task-progress wakeups.
    func configurePushNotifications() async {
        let center = UNUserNotificationCenter.current()
        do {
            var settings = await center.notificationSettings()
            if settings.authorizationStatus == .notDetermined {
                _ = try await center.requestAuthorization(options: [.alert, .sound, .badge])
                settings = await center.notificationSettings()
            }
            switch settings.authorizationStatus {
            case .authorized, .provisional, .ephemeral:
                UIApplication.shared.registerForRemoteNotifications()
            case .denied, .notDetermined:
                break
            @unknown default:
                break
            }
        } catch {
            logger.error("push_authorization_failed error=\(error.localizedDescription, privacy: .public)")
        }
    }

    func receivePushToken(_ token: String) {
        storage.set(token, forKey: StorageKey.apnsDeviceToken.rawValue)
        Task { await syncStoredPushTokenIfPossible() }
    }

    func syncStoredPushTokenIfPossible() async {
        guard let identity, let selectedAgent else { return }
        do {
            let sessionToken = try await authenticate(identity: identity, agent: selectedAgent)
            controller?.updateSessionToken(sessionToken)
            try await syncStoredPushToken(identity: identity)
        } catch {
            logger.error("push_token_sync_failed error=\(error.localizedDescription, privacy: .public)")
        }
    }

    func syncStoredPushToken(identity: LocalDeviceIdentity) async throws {
        guard let pushToken = storedPushToken else { return }
        let controller = try requireController()
        _ = try await controller.updatePushToken(
            request: PushTokenUpdateRequestData(
                deviceID: identity.deviceID,
                pushToken: pushToken
            )
        )
    }

    func enterThreadDetail(_ threadID: String) {
        visibleThreadID = threadID
    }

    func leaveThreadDetail(_ threadID: String) {
        if visibleThreadID == threadID {
            visibleThreadID = nil
        }
    }

    func shouldPresentPushNotification(userInfo: [AnyHashable: Any]) async -> Bool {
        guard (userInfo["kind"] as? String) == "task_progress" else {
            return true
        }
        do {
            let decoded = try decodeTaskProgressNotification(userInfo: userInfo)
            return visibleThreadID != decoded.threadID
        } catch {
            logger.error("push_presentation_decode_failed error=\(error.localizedDescription, privacy: .public)")
            return true
        }
    }

    /// Handles an APNs tap by decrypting the opaque locator and refreshing the target thread.
    func handlePushNotification(userInfo: [AnyHashable: Any]) async {
        guard (userInfo["kind"] as? String) == "task_progress",
              userInfo["ciphertext"] is String
        else {
            return
        }
        do {
            let decoded = try decodeTaskProgressNotification(userInfo: userInfo)
            selectedAgentID = decoded.agent.agentID
            persistSelection()
            pendingPushThreadRoute = PushThreadRoute(threadID: decoded.threadID)
            await refresh()
            await refreshThreadDetails(threadID: decoded.threadID)
        } catch {
            pendingError = error.localizedDescription
        }
    }

    private func decodeTaskProgressNotification(
        userInfo: [AnyHashable: Any]
    ) throws -> (threadID: String, agent: PairedAgent) {
        guard let ciphertext = userInfo["ciphertext"] as? String else {
            throw AppModelError.invalidPairPayload
        }
        let identity = try ensurePairingIdentity()
        let agent = try agentForPush(userInfo: userInfo)
        let cryptoContext = try identityService.makePayloadCryptoContext(
            peerPublicKey: agent.agentEncryptionPublicKey,
            bindingID: agent.bindingID
        )
        let plaintext = try PayloadCryptoService.decrypt(
            envelope: ciphertext,
            context: cryptoContext,
            direction: .agentToIOS,
            additionalData: LiveRealtimeEventDecoder.taskProgressPushAdditionalData(
                deviceID: identity.deviceID,
                agentID: agent.agentID
            )
        )
        let payload = try JSONDecoder().decode(TaskProgressNotificationPayload.self, from: plaintext)
        return (payload.threadID, agent)
    }

    private func agentForPush(userInfo: [AnyHashable: Any]) throws -> PairedAgent {
        if let agentID = userInfo["agent_id"] as? String,
           let agent = pairedAgents.first(where: { $0.agentID == agentID }) {
            return agent
        }
        if let selectedAgent {
            return selectedAgent
        }
        throw AppModelError.invalidPairPayload
    }
}
