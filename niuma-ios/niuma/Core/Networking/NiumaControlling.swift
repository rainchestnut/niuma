import Foundation

@MainActor
protocol NiumaControlling: AnyObject {
    var modeLabel: String { get }

    /// Applies the latest authenticated server token to future requests.
    /// - Parameters:
    ///   - token: Optional session token returned by `/auth/verify`; `nil` clears authenticated state.
    func updateSessionToken(_ token: String?)

    /// Requests a one-time authentication challenge for a device.
    /// - Parameters:
    ///   - deviceID: Mobile device id that must already be registered on the server.
    /// - Returns: `ChallengePayload` containing the challenge id, challenge text, and expiry.
    func issueChallenge(deviceID: String) async throws -> ChallengePayload

    /// Verifies a signed challenge response and receives a session token.
    /// - Parameters:
    ///   - request: `VerifyRequestData` containing device id, challenge id, nonce, timestamp, and signature.
    /// - Returns: `SessionTokenPayload` used to authorize HTTP metadata calls and WebSocket connections.
    func verify(request: VerifyRequestData) async throws -> SessionTokenPayload

    /// Confirms pairing with a desktop gateway using the scanned QR payload.
    /// - Parameters:
    ///   - request: `PairConfirmRequestData` carrying ids, pair token, encrypted handshake, and pair signature.
    /// - Returns: `PairConfirmResponseData` with the paired agent metadata.
    func confirmPairing(request: PairConfirmRequestData) async throws -> PairConfirmResponseData

    /// Revokes an existing mobile-to-desktop pairing.
    /// - Parameters:
    ///   - request: `PairRevokeRequestData` identifying the device and agent binding to remove.
    /// - Returns: `PairRevokeResponseData` indicating whether the binding was removed.
    func revokePairing(request: PairRevokeRequestData) async throws -> PairRevokeResponseData

    /// Updates the APNs token used for encrypted wakeup notifications.
    /// - Parameters:
    ///   - request: `PushTokenUpdateRequestData` carrying the local device id and APNs token.
    /// - Returns: Server acknowledgement that the token was stored for this iOS device.
    func updatePushToken(request: PushTokenUpdateRequestData) async throws -> PushTokenUpdateResponseData

    /// Reads the current desktop gateway payload from the local pairing page.
    /// - Returns: `PairCodePayload` identical to the QR payload shown by the gateway dashboard.
    func fetchDesktopPairingPayload() async throws -> PairCodePayload

    /// Opens the realtime WebSocket for the paired device and agent.
    /// - Parameters:
    ///   - deviceID: Registered iOS device id that owns the connection.
    ///   - agentID: Paired desktop agent id that should receive mobile-originated messages.
    ///   - sessionToken: Authenticated session token included in the WebSocket query string.
    /// - Returns: Async stream of decoded realtime events from niuma-server.
    func connectRealtime(deviceID: String, agent: PairedAgent, sessionToken: String) async throws -> AsyncThrowingStream<RealtimeEvent, Error>

    /// Closes the active realtime WebSocket, if one is connected.
    func disconnectRealtime()

    /// Requests a desktop metadata refresh through the realtime channel.
    /// - Parameters:
    ///   - request: `MetadataRefreshRequestData` carrying the refresh request id.
    func requestMetadataRefresh(request: MetadataRefreshRequestData) async throws

    /// Requests current Git branch/worktree changes for one Codex thread.
    /// - Parameters:
    ///   - request: Thread id plus optional base ref used by the desktop gateway.
    func requestBranchChanges(request: BranchChangesRequestData) async throws

    /// Sends a user task-start request to the paired desktop agent.
    /// - Parameters:
    ///   - request: `TaskStartRequestData` containing routing ids, payload ciphertext, and mobile signature.
    func sendTaskStart(request: TaskStartRequestData) async throws

    /// Requests desktop replay for an existing thread.
    /// - Parameters:
    ///   - request: `ResumeThreadRequestData` containing the target thread and current mobile cursor.
    func resumeThread(request: ResumeThreadRequestData) async throws

    /// Sends an approval decision selected on the mobile device.
    /// - Parameters:
    ///   - request: `ApprovalDecisionRequestData` carrying approval id and decision.
    func respondToApproval(request: ApprovalDecisionRequestData) async throws

    /// Sends answers for a structured user-input request.
    /// - Parameters:
    ///   - request: `UserInputResponseRequestData` carrying request id and keyed answer values.
    func respondToUserInput(request: UserInputResponseRequestData) async throws

    /// Ensures a content-addressed relay record exists for an encrypted attachment transfer.
    /// - Parameters:
    ///   - transferID: SHA-256 content hash used as the relay identifier.
    ///   - request: `TransferEnsureRequestData` containing routing metadata and encrypted size.
    /// - Returns: Relay TTL plus whether the payload bytes still need to be uploaded.
    func ensureTransfer(transferID: String, request: TransferEnsureRequestData) async throws -> TransferEnsureResponseData

    /// Uploads the complete encrypted payload to the payload-blind relay.
    /// - Parameters:
    ///   - transferID: SHA-256 content hash used as the relay identifier.
    ///   - body: Complete encrypted payload bytes for this transfer.
    ///   - deviceID: Registered device id included in the relay auth header.
    /// - Returns: Upload acknowledgement with the refreshed relay TTL.
    func uploadTransfer(transferID: String, body: Data, deviceID: String) async throws -> TransferUploadResponseData

    /// Downloads an encrypted transfer payload from the relay.
    /// - Parameters:
    ///   - transferID: SHA-256 content hash used as the relay identifier.
    ///   - deviceID: Registered device id included in the relay auth header.
    /// - Returns: Raw encrypted payload bytes; plaintext metadata remains outside the server.
    func downloadTransfer(transferID: String, deviceID: String) async throws -> Data

    /// Acknowledges that a content-addressed transfer has been received by the device.
    /// - Parameters:
    ///   - transferID: SHA-256 content hash used as the relay identifier.
    ///   - request: `TransferAckRequestData` naming the acknowledging device.
    /// - Returns: Relay acknowledgement response.
    func ackTransfer(transferID: String, request: TransferAckRequestData) async throws -> TransferAckResponseData
}
