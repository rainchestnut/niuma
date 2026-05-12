import Foundation
import OSLog

nonisolated struct NiumaHTTPError: LocalizedError {
    let statusCode: Int
    let detail: String

    var errorDescription: String? {
        detail
    }
}

private nonisolated struct ServerErrorBody: Decodable {
    let detail: String
}

@MainActor
final class LiveNiumaController: NiumaControlling {
    let modeLabel: String

    private let baseURL: URL
    private let session: URLSession
    private let decoder: JSONDecoder
    private let encoder: JSONEncoder
    private var webSocketTask: URLSessionWebSocketTask?
    private let identityService: DeviceIdentityService
    private let logger = Logger(subsystem: "com.rainchestnut.niuma", category: "transport")
    private var sessionToken: String?

    init(
        baseURL: URL,
        identityService: DeviceIdentityService,
        session: URLSession = .shared
    ) {
        self.baseURL = baseURL
        self.identityService = identityService
        self.session = session
        self.modeLabel = baseURL.absoluteString
        self.decoder = JSONDecoder()
        self.encoder = JSONEncoder()
        decoder.dateDecodingStrategy = .iso8601
        encoder.dateEncodingStrategy = .iso8601
    }

    func updateSessionToken(_ token: String?) {
        self.sessionToken = token
    }

    func issueChallenge(deviceID: String) async throws -> ChallengePayload {
        let response: ChallengeResponseData = try await send(
            .post,
            path: "/auth/challenge",
            body: ChallengeRequestData(deviceID: deviceID)
        )
        return ChallengePayload(
            challengeID: response.challengeID,
            challenge: response.challenge,
            expiresAt: response.expiresAt
        )
    }

    /// Verifies a signed challenge and stores the returned session token.
    func verify(request: VerifyRequestData) async throws -> SessionTokenPayload {
        let response: VerifyResponseData = try await send(.post, path: "/auth/verify", body: request)
        return SessionTokenPayload(sessionToken: response.sessionToken)
    }

    func confirmPairing(request: PairConfirmRequestData) async throws -> PairConfirmResponseData {
        try await send(.post, path: "/pair/confirm", body: request)
    }

    func revokePairing(request: PairRevokeRequestData) async throws -> PairRevokeResponseData {
        try await send(.post, path: "/pair/revoke", body: request)
    }

    func updatePushToken(request: PushTokenUpdateRequestData) async throws -> PushTokenUpdateResponseData {
        try await send(.post, path: "/devices/push-token", body: request)
    }

    /// Opens the authenticated WebSocket and continuously consumes server events for the selected agent.
    func connectRealtime(deviceID: String, agent: PairedAgent, sessionToken: String) async throws -> AsyncThrowingStream<RealtimeEvent, Error> {
        webSocketTask?.cancel(with: .goingAway, reason: nil)
        var components = URLComponents(url: websocketBaseURL.appending(path: "/ws/mobile"), resolvingAgainstBaseURL: false)
        components?.queryItems = [
            URLQueryItem(name: "device_id", value: deviceID),
            URLQueryItem(name: "agent_id", value: agent.agentID),
            URLQueryItem(name: "session_token", value: sessionToken)
        ]
        guard let url = components?.url else {
            throw URLError(.badURL)
        }

        let task = session.webSocketTask(with: url)
        task.resume()
        webSocketTask = task
        logger.info("mobile_ws_connect device_id=\(deviceID, privacy: .public) agent_id=\(agent.agentID, privacy: .public)")
        let realtimeLogger = logger
        let cryptoContext = try identityService.makePayloadCryptoContext(
            peerPublicKey: agent.agentEncryptionPublicKey,
            bindingID: agent.bindingID
        )
        let agentID = agent.agentID

        return AsyncThrowingStream { continuation in
            let reader = Task.detached(priority: .userInitiated) {
                let eventDecoder = JSONDecoder()
                eventDecoder.dateDecodingStrategy = .iso8601
                do {
                    while !Task.isCancelled {
                        let message = try await task.receive()
                        guard case .string(let payload) = message else { continue }
                        do {
                            let event = try LiveRealtimeEventDecoder.decode(
                                from: payload,
                                agentID: agentID,
                                cryptoContext: cryptoContext,
                                decoder: eventDecoder,
                                logger: realtimeLogger
                            )
                            continuation.yield(event)
                        } catch {
                            // Keep the websocket alive if one server/gateway payload is
                            // newer than this mobile build can decode. A later refresh can
                            // still replay source-of-truth thread entries.
                            realtimeLogger.error("mobile_ws_decode_failed error=\(error.localizedDescription, privacy: .public)")
                            continue
                        }
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }

            continuation.onTermination = { _ in
                reader.cancel()
                task.cancel(with: .normalClosure, reason: nil)
            }
        }
    }

    /// Closes the current WebSocket task without touching cached metadata.
    func disconnectRealtime() {
        logger.info("mobile_ws_disconnect")
        webSocketTask?.cancel(with: .goingAway, reason: nil)
        webSocketTask = nil
    }

    func sendTaskStart(request: TaskStartRequestData) async throws {
        let businessPayload = ContentPartsPayload(
            contentParts: request.contentParts ?? [ContentPart(kind: .text, text: request.prompt)]
        )
        let cryptoContext = try identityService.makePayloadCryptoContext(
            peerPublicKey: request.agentEncryptionPublicKey,
            bindingID: request.bindingID
        )
        let ciphertext = try PayloadCryptoService.encrypt(
            plaintext: encoder.encode(businessPayload),
            context: cryptoContext,
            direction: .iosToAgent,
            additionalData: LiveRealtimeEventDecoder.taskStartAdditionalData(request)
        )
        let signature = try identityService.makeTaskStartSignature(
            deviceID: request.deviceID,
            agentID: request.agentID,
            projectID: request.projectID,
            threadID: request.threadID,
            ciphertext: ciphertext
        )
        let taskStart = LiveTaskStartMessage(
            kind: "task_start",
            deviceID: request.deviceID,
            agentID: request.agentID,
            projectID: request.projectID,
            threadID: request.threadID,
            ciphertext: ciphertext,
            signature: signature,
            model: request.model,
            effort: request.effort,
            approvalPolicy: request.approvalPolicy,
            approvalsReviewer: request.approvalsReviewer,
            sandboxMode: request.sandboxMode
        )
        try await sendWebSocket(taskStart)
    }

    func sendTaskSteer(request: TaskSteerRequestData) async throws {
        let businessPayload = ContentPartsPayload(
            contentParts: request.contentParts ?? [ContentPart(kind: .text, text: request.prompt)]
        )
        let cryptoContext = try identityService.makePayloadCryptoContext(
            peerPublicKey: request.agentEncryptionPublicKey,
            bindingID: request.bindingID
        )
        let ciphertext = try PayloadCryptoService.encrypt(
            plaintext: encoder.encode(businessPayload),
            context: cryptoContext,
            direction: .iosToAgent,
            additionalData: LiveRealtimeEventDecoder.taskSteerAdditionalData(request)
        )
        let signature = try identityService.makeTaskSteerSignature(
            deviceID: request.deviceID,
            agentID: request.agentID,
            threadID: request.threadID,
            ciphertext: ciphertext
        )
        try await sendWebSocket(
            LiveTaskSteerMessage(
                kind: "task_steer",
                deviceID: request.deviceID,
                agentID: request.agentID,
                threadID: request.threadID,
                ciphertext: ciphertext,
                signature: signature
            )
        )
    }

    func interruptTask(request: TaskInterruptRequestData) async throws {
        let signature = try identityService.makeTaskInterruptSignature(
            deviceID: request.deviceID,
            agentID: request.agentID,
            threadID: request.threadID
        )
        try await sendWebSocket(
            LiveTaskInterruptMessage(
                kind: "task_interrupt",
                deviceID: request.deviceID,
                agentID: request.agentID,
                threadID: request.threadID,
                signature: signature
            )
        )
    }

    func requestMetadataRefresh(request: MetadataRefreshRequestData) async throws {
        logger.info("mobile_ws_metadata_refresh_send request_id=\(request.requestID, privacy: .public)")
        try await sendWebSocket(
            LiveMetadataRefreshMessage(
                kind: "metadata_refresh",
                requestID: request.requestID,
                deviceID: request.deviceID
            )
        )
    }

    func requestBranchChanges(request: BranchChangesRequestData) async throws {
        logger.info("mobile_ws_branch_changes_send request_id=\(request.requestID, privacy: .public) thread_id=\(request.threadID, privacy: .public)")
        try await sendWebSocket(
            LiveBranchChangesRequestMessage(
                kind: "branch_changes_request",
                requestID: request.requestID,
                deviceID: request.deviceID,
                threadID: request.threadID,
                baseRef: request.baseRef
            )
        )
    }

    func requestThreadArchive(request: ThreadArchiveRequestData) async throws {
        logger.info("mobile_ws_thread_archive_send request_id=\(request.requestID, privacy: .public) thread_id=\(request.threadID, privacy: .public)")
        try await sendWebSocket(
            LiveThreadArchiveRequestMessage(
                kind: "thread_archive_request",
                requestID: request.requestID,
                deviceID: request.deviceID,
                threadID: request.threadID
            )
        )
    }

    func requestThreadRename(request: ThreadRenameRequestData) async throws {
        logger.info("mobile_ws_thread_rename_send request_id=\(request.requestID, privacy: .public) thread_id=\(request.threadID, privacy: .public)")
        try await sendWebSocket(
            LiveThreadRenameRequestMessage(
                kind: "thread_rename_request",
                requestID: request.requestID,
                deviceID: request.deviceID,
                threadID: request.threadID,
                title: request.title
            )
        )
    }

    func resumeThread(request: ResumeThreadRequestData) async throws {
        let payload = LiveResumeThreadMessage(
            kind: "resume_thread",
            threadID: request.threadID,
            cursor: request.cursor,
            checkpoint: request.checkpoint
        )
        logger.info("mobile_ws_resume_thread_send thread_id=\(request.threadID, privacy: .public) cursor=\(request.cursor, privacy: .public) checkpoint_present=\((request.checkpoint != nil), privacy: .public)")
        try await sendWebSocket(payload)
    }

    func respondToApproval(request: ApprovalDecisionRequestData) async throws {
        let cryptoContext = try identityService.makePayloadCryptoContext(
            peerPublicKey: request.agentEncryptionPublicKey,
            bindingID: request.bindingID
        )
        let ciphertext = try PayloadCryptoService.encrypt(
            plaintext: encoder.encode(
                ApprovalResponsePlaintext(
                    approvalID: request.approvalID,
                    decision: request.decision.rawValue,
                    grantScope: request.grantScope
                )
            ),
            context: cryptoContext,
            direction: .iosToAgent,
            additionalData: LiveRealtimeEventDecoder.mobileResponseAdditionalData(
                kind: "approval_response",
                deviceID: request.deviceID,
                agentID: request.agentID
            )
        )
        let payload = LiveApprovalResponseMessage(
            kind: "approval_response",
            approvalID: request.approvalID,
            ciphertext: ciphertext
        )
        try await sendWebSocket(payload)
    }

    func respondToUserInput(request: UserInputResponseRequestData) async throws {
        let cryptoContext = try identityService.makePayloadCryptoContext(
            peerPublicKey: request.agentEncryptionPublicKey,
            bindingID: request.bindingID
        )
        let ciphertext = try PayloadCryptoService.encrypt(
            plaintext: encoder.encode(
                UserInputResponsePlaintext(
                    requestID: request.requestID,
                    answers: request.answers.mapValues { LiveUserInputAnswerMessage(answers: $0) }
                )
            ),
            context: cryptoContext,
            direction: .iosToAgent,
            additionalData: LiveRealtimeEventDecoder.mobileResponseAdditionalData(
                kind: "user_input_response",
                deviceID: request.deviceID,
                agentID: request.agentID
            )
        )
        let payload = LiveUserInputResponseMessage(
            kind: "user_input_response",
            requestID: request.requestID,
            ciphertext: ciphertext
        )
        try await sendWebSocket(payload)
    }

    /// Ensures Transfer.
    func ensureTransfer(transferID: String, request: TransferEnsureRequestData) async throws -> TransferEnsureResponseData {
        try await send(.post, path: "/transfers/\(transferID)/ensure", body: request)
    }

    /// Uploads the complete transfer body.
    func uploadTransfer(
        transferID: String,
        body: Data,
        deviceID: String
    ) async throws -> TransferUploadResponseData {
        try await sendBinary(
            .put,
            path: "/transfers/\(transferID)",
            body: body,
            headers: ["X-Device-ID": deviceID]
        )
    }

    func downloadTransfer(transferID: String, deviceID: String) async throws -> Data {
        try await sendData(
            .get,
            path: "/transfers/\(transferID)",
            queryItems: [URLQueryItem(name: "device_id", value: deviceID)]
        )
    }

    func ackTransfer(
        transferID: String,
        request: TransferAckRequestData
    ) async throws -> TransferAckResponseData {
        try await send(.post, path: "/transfers/\(transferID)/ack", body: request)
    }

    /// Sends a JSON HTTP request and decodes the JSON response.
    private func send<Response: Decodable, Body: Encodable>(
        _ method: HTTPMethod,
        path: String,
        queryItems: [URLQueryItem] = [],
        body: Body? = nil,
        extraHeaders: [String: String] = [:]
    ) async throws -> Response {
        var components = URLComponents(url: baseURL.appending(path: path), resolvingAgainstBaseURL: false)
        components?.queryItems = queryItems.isEmpty ? nil : queryItems
        guard let url = components?.url else {
            throw URLError(.badURL)
        }

        var request = URLRequest(url: url)
        request.httpMethod = method.rawValue
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        if let sessionToken {
            request.setValue(sessionToken, forHTTPHeaderField: "X-Session-Token")
        }
        for (name, value) in extraHeaders {
            request.setValue(value, forHTTPHeaderField: name)
        }
        if let body {
            request.httpBody = try encoder.encode(body)
        }

        let (data, response) = try await session.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw URLError(.badServerResponse)
        }
        guard (200..<300).contains(httpResponse.statusCode) else {
            throw Self.httpError(statusCode: httpResponse.statusCode, data: data)
        }
        return try decoder.decode(Response.self, from: data)
    }

    /// Sends a JSON-free HTTP request and decodes the JSON response.
    private func send<Response: Decodable>(
        _ method: HTTPMethod,
        path: String,
        queryItems: [URLQueryItem] = []
    ) async throws -> Response {
        let body: EmptyBody? = nil
        return try await send(method, path: path, queryItems: queryItems, body: body)
    }

    /// Sends a binary HTTP request used by whole-payload transfer uploads.
    private func sendBinary<Response: Decodable>(
        _ method: HTTPMethod,
        path: String,
        body: Data,
        headers: [String: String]
    ) async throws -> Response {
        var request = try request(method, path: path)
        request.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
        request.httpBody = body
        for (name, value) in headers {
            request.setValue(value, forHTTPHeaderField: name)
        }

        let (data, response) = try await session.data(for: request)
        try validate(response: response, data: data)
        return try decoder.decode(Response.self, from: data)
    }

    /// Sends an HTTP request that expects raw bytes instead of JSON.
    private func sendData(
        _ method: HTTPMethod,
        path: String,
        queryItems: [URLQueryItem]
    ) async throws -> Data {
        var request = try request(method, path: path, queryItems: queryItems)
        request.setValue("application/octet-stream", forHTTPHeaderField: "Accept")
        let (data, response) = try await session.data(for: request)
        try validate(response: response, data: data)
        return data
    }

    /// Builds an authenticated HTTP request for niuma-server.
    private func request(
        _ method: HTTPMethod,
        path: String,
        queryItems: [URLQueryItem] = []
    ) throws -> URLRequest {
        var components = URLComponents(url: baseURL.appending(path: path), resolvingAgainstBaseURL: false)
        components?.queryItems = queryItems.isEmpty ? nil : queryItems
        guard let url = components?.url else {
            throw URLError(.badURL)
        }
        var request = URLRequest(url: url)
        request.httpMethod = method.rawValue
        if let sessionToken {
            request.setValue(sessionToken, forHTTPHeaderField: "X-Session-Token")
        }
        return request
    }

    /// Validates an HTTP response and exposes server error text when available.
    private func validate(response: URLResponse, data: Data) throws {
        guard let httpResponse = response as? HTTPURLResponse else {
            throw URLError(.badServerResponse)
        }
        guard (200..<300).contains(httpResponse.statusCode) else {
            throw Self.httpError(statusCode: httpResponse.statusCode, data: data)
        }
    }

    /// Converts FastAPI error bodies into typed transport errors used by app state recovery.
    /// - Parameters:
    ///   - statusCode: HTTP status code returned by niuma-server.
    ///   - data: Raw response body, usually `{"detail": ...}` from FastAPI.
    /// - Returns: `NiumaHTTPError` with a user-readable detail string.
    private nonisolated static func httpError(statusCode: Int, data: Data) -> NiumaHTTPError {
        let detail = (try? JSONDecoder().decode(ServerErrorBody.self, from: data).detail)
            ?? String(data: data, encoding: .utf8)
            ?? "unknown error"
        return NiumaHTTPError(statusCode: statusCode, detail: detail)
    }

    private var websocketBaseURL: URL {
        guard var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false) else {
            return baseURL
        }
        switch components.scheme?.lowercased() {
        case "https":
            components.scheme = "wss"
        default:
            components.scheme = "ws"
        }
        return components.url ?? baseURL
    }

    /// Serializes and sends one payload-blind WebSocket message.
    private func sendWebSocket<Payload: Encodable>(_ payload: Payload) async throws {
        guard let webSocketTask else {
            throw AppModelError.realtimeNotConnected
        }
        let data = try encoder.encode(payload)
        guard let jsonString = String(data: data, encoding: .utf8) else {
            throw NSError(domain: "NiumaEncodingError", code: 0, userInfo: [
                NSLocalizedDescriptionKey: "failed to encode websocket payload"
            ])
        }
        try await webSocketTask.send(.string(jsonString))
    }

}
