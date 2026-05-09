import Foundation
import Observation
import OSLog
import SwiftUI

@MainActor
@Observable
final class AppModel {
    var deviceState: DeviceLifecycleState = .uninitialized
    var connectionState: RealtimeConnectionState = .disconnected
    var runtimeState: ThreadRuntimeState = .idle
    var identity: LocalDeviceIdentity?
    var pairedAgents: [PairedAgent] = []
    var selectedAgentID: String?
    var projects: [ProjectSummary] = []
    var threadsByProject: [String: [ThreadSummary]] = [:]
    var approvals: [ApprovalSummary] = []
    var approvalResponseFailures: [String: String] = [:]
    var userInputRequests: [UserInputRequestSummary] = []
    var timelines: [String: ThreadTimeline] = [:]
    var threadRefreshStates: [String: ThreadRefreshStatus] = [:]
    var transientOutgoingEntries: [String: [ThreadEntry]] = [:]
    var localAttachments: [String: LocalAttachmentPayload] = [:]
    var branchChangesByThread: [String: BranchChangesResult] = [:]
    var archivingThreadIDs: Set<String> = []
    var availableModels: [String] = []
    var selectedModelID: String?
    var selectedReasoningEffort: ReasoningEffort
    var approvalPermissionPreset: ApprovalPermissionPreset
    var customApprovalPolicy: CodexApprovalPolicy
    var customApprovalsReviewer: CodexApprovalsReviewer
    var customSandboxMode: CodexSandboxMode
    var pendingError: String?
    var isBootstrapping = false
    var isRefreshing = false
    var isUpdatingServerBaseURL = false
    var visibleThreadID: String?
    var appLanguage: AppLanguage
    var appTheme: AppTheme
    var serverBaseURLText: String
    var serverBaseURLValidationMessage: String?
    var pendingPushThreadRoute: PushThreadRoute?

    @ObservationIgnored var controller: (any NiumaControlling)?
    @ObservationIgnored let controllerFactory: (URL) -> any NiumaControlling
    @ObservationIgnored let identityService: DeviceIdentityService
    @ObservationIgnored let storage: UserDefaultsStore
    @ObservationIgnored let dataStore: NiumaDataStore
    @ObservationIgnored let threadSyncPipeline: ThreadSyncPipeline
    @ObservationIgnored let realtimeSendTimeout: Duration
    @ObservationIgnored var realtimeTask: Task<Void, Never>?
    @ObservationIgnored var threadSyncResultTask: Task<Void, Never>?
    @ObservationIgnored var threadRefreshTimeoutTasks: [String: Task<Void, Never>] = [:]
    @ObservationIgnored var hasBootstrapped = false
    @ObservationIgnored let logger = Logger(subsystem: "com.rainchestnut.niuma", category: "realtime")
    @ObservationIgnored var serverBaseURL: URL?

    init(
        serverBaseURL: URL?,
        controllerFactory: @escaping (URL) -> any NiumaControlling,
        identityService: DeviceIdentityService,
        storage: UserDefaultsStore,
        dataStore: NiumaDataStore,
        threadSyncPipeline: ThreadSyncPipeline,
        realtimeSendTimeout: Duration = .seconds(8)
    ) {
        self.serverBaseURL = serverBaseURL
        self.serverBaseURLText = serverBaseURL.map(Self.displayString(forServerBaseURL:)) ?? ""
        self.controllerFactory = controllerFactory
        self.controller = serverBaseURL.map(controllerFactory)
        self.identityService = identityService
        self.storage = storage
        self.dataStore = dataStore
        self.threadSyncPipeline = threadSyncPipeline
        self.realtimeSendTimeout = realtimeSendTimeout
        let persistedAgents = dataStore.loadAgents()
        self.pairedAgents = persistedAgents.agents
        self.selectedAgentID = persistedAgents.selectedAgentID
        self.projects = dataStore.loadProjects()
        self.threadsByProject = dataStore.loadThreadsByProject()
        self.localAttachments = dataStore.loadLocalAttachments()
        self.timelines = [:]
        self.selectedModelID = storage.string(forKey: StorageKey.selectedModelID.rawValue)
        self.selectedReasoningEffort = ReasoningEffort(
            rawValue: storage.string(forKey: StorageKey.selectedReasoningEffort.rawValue) ?? ""
        ) ?? .high
        self.approvalPermissionPreset = ApprovalPermissionPreset(
            rawValue: storage.string(forKey: StorageKey.approvalPermissionPreset.rawValue) ?? ""
        ) ?? .defaultPermissions
        self.customApprovalPolicy = CodexApprovalPolicy(
            rawValue: storage.string(forKey: StorageKey.customApprovalPolicy.rawValue) ?? ""
        ) ?? .onRequest
        self.customApprovalsReviewer = CodexApprovalsReviewer(
            rawValue: storage.string(forKey: StorageKey.customApprovalsReviewer.rawValue) ?? ""
        ) ?? .user
        self.customSandboxMode = CodexSandboxMode(
            rawValue: storage.string(forKey: StorageKey.customSandboxMode.rawValue) ?? ""
        ) ?? .workspaceWrite
        self.appLanguage = AppLanguage(rawValue: storage.string(forKey: StorageKey.appLanguage.rawValue) ?? "") ?? .chinese
        self.appTheme = AppTheme(rawValue: storage.string(forKey: StorageKey.appTheme.rawValue) ?? "") ?? .system
        self.threadSyncResultTask = Task { [weak self, results = threadSyncPipeline.results] in
            for await pipelineResults in results {
                self?.applyPipelineResults(pipelineResults)
            }
        }
    }

    var selectedAgent: PairedAgent? {
        guard let selectedAgentID else { return pairedAgents.first }
        return pairedAgents.first(where: { $0.agentID == selectedAgentID }) ?? pairedAgents.first
    }

    var recentSessions: [ThreadSummary] {
        conversationSessions.prefix(6).map(\.self)
    }

    var workspaceProjects: [ProjectSummary] {
        projects.filter { $0.projectID != conversationProjectID }
    }

    var conversationSessions: [ThreadSummary] {
        threadsByProject.values
            .flatMap { $0 }
            .filter { $0.status != .archived }
            .filter { $0.projectID == conversationProjectID }
            .sorted(by: { $0.updatedAt > $1.updatedAt })
            .map(\.self)
    }

    var inboxSessions: [ThreadSummary] {
        conversationSessions.prefix(8).map(\.self)
    }

    var visibleProjects: [ProjectSummary] {
        workspaceProjects.filter { project in
            (threadsByProject[project.projectID] ?? []).contains { $0.status != .archived }
        }
    }

    var activeThreadCount: Int {
        threadsByProject.values.flatMap { $0 }.filter { $0.status == .running || $0.status == .waitingApproval }.count
    }

    var pendingApprovalCount: Int {
        approvals.filter { $0.status == .pending }.count
    }

    var pendingUserInputCount: Int {
        userInputRequests.filter { $0.status == .pending }.count
    }

    var displayedModelID: String {
        selectedModelID ?? availableModels.first ?? localized("model.placeholder")
    }

    var hasPairedAgent: Bool {
        !pairedAgents.isEmpty
    }

    var controllerModeLabel: String {
        controller?.modeLabel ?? localized("settings.server.not_configured")
    }

    var serverBaseURLPlaceholder: String {
        localized("settings.server.placeholder")
    }

    var localeIdentifier: String {
        appLanguage.localeIdentifier
    }

    var preferredColorScheme: ColorScheme? {
        switch appTheme {
        case .system:
            return nil
        case .light:
            return .light
        case .dark:
            return .dark
        }
    }

    var selectedProjectID: String? {
        workspaceProjects.first?.projectID
    }

    func bootstrapIfNeeded() async {
        guard !hasBootstrapped else { return }
        hasBootstrapped = true
        await bootstrap()
    }

    /// Bootstraps identity, cached state, selected agent authentication, and realtime setup.
    func bootstrap() async {
        isBootstrapping = true
        defer { isBootstrapping = false }

        do {
            let identity = try identityService.ensureIdentity(deviceName: DeviceIdentityService.defaultDeviceName)
            self.identity = identity
            await configurePushNotifications()
            deviceState = pairedAgents.isEmpty ? .unpaired : .paired
            if selectedAgentID == nil {
                selectedAgentID = pairedAgents.first?.agentID
                persistSelection()
            }
            if selectedAgent != nil {
                await refresh()
            }
        } catch {
            pendingError = error.localizedDescription
            deviceState = .uninitialized
        }
    }

    /// Refreshes visible metadata from the selected desktop agent.
    func refresh() async {
        guard let identity, let selectedAgent else { return }
        isRefreshing = true
        defer { isRefreshing = false }

        do {
            let metadataRequestID = UUID().uuidString.lowercased()
            try await sendRealtimeRequestWithRecovery(
                identity: identity,
                agent: selectedAgent,
                operationLabel: "metadata_refresh",
                forceReconnect: true,
                afterAuthentication: { [self] _, _ in
                    try await syncStoredPushToken(identity: identity)
                }
            ) { controller in
                try await controller.requestMetadataRefresh(
                    request: MetadataRefreshRequestData(
                        requestID: metadataRequestID,
                        deviceID: identity.deviceID
                    )
                )
            }
            // List metadata now has a single source: realtime sync pushed by
            // the desktop gateway. Cached SwiftData rows stay visible until the
            // gateway streams fresher project/session/thread/approval/model rows.
            updateAgentOnline(agentID: selectedAgent.agentID, isOnline: true)
            deviceState = .paired
        } catch {
            if isServerForgotCurrentDevice(error, deviceID: identity.deviceID) {
                resetPairingAfterServerIdentityLoss()
                pendingError = AppModelError.serverForgotDevice.localizedDescription
                return
            }
            if !isTransientRealtimeDisconnect(error) {
                pendingError = error.localizedDescription
            }
            updateAgentOnline(agentID: selectedAgent.agentID, isOnline: false)
            connectionState = .degraded
        }
    }

    /// Returns visible sessions for the requested project.
}
