import Foundation
import SwiftUI

private enum ApprovalResolutionAction: Equatable {
    case allowTurn
    case allowSession
    case reject

    var title: String {
        switch self {
        case .allowTurn:
            return "本次允许"
        case .allowSession:
            return "本会话允许"
        case .reject:
            return "拒绝"
        }
    }

    var submittedTitle: String {
        switch self {
        case .allowTurn:
            return "正在允许本次..."
        case .allowSession:
            return "正在允许本会话..."
        case .reject:
            return "正在拒绝..."
        }
    }
}

/// Detail screen for an approval request attached to a thread timeline.
struct ApprovalDetailView: View {
    @Environment(AppModel.self) private var appModel
    let approval: ApprovalSummary
    @State private var submittedAction: ApprovalResolutionAction?
    @State private var localErrorMessage: String?

    private var currentApproval: ApprovalSummary {
        appModel.currentApproval(approval.approvalID) ?? approval
    }

    private var isResolved: Bool {
        currentApproval.status == .resolved
    }

    private var isWaitingForConfirmation: Bool {
        submittedAction != nil && !isResolved && failureMessage == nil
    }

    private var failureMessage: String? {
        localErrorMessage ?? appModel.approvalFailureMessage(for: approval.approvalID)
    }

    var body: some View {
        let presentation = ApprovalPresentation(approval: currentApproval)
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                SurfaceCard(title: "审批类型") {
                    ApprovalTypeSummaryView(presentation: presentation)
                }

                SurfaceCard(title: "请求内容") {
                    VStack(alignment: .leading, spacing: 12) {
                        ApprovalReadableRow(
                            title: "要执行的动作",
                            value: presentation.actionText,
                            monospaced: presentation.actionMonospaced
                        )
                        ApprovalReadableRow(
                            title: "原因",
                            value: presentation.reasonText ?? "未提供原因"
                        )
                        ForEach(presentation.contextRows) { row in
                            ApprovalReadableRow(
                                title: row.title,
                                value: row.value,
                                monospaced: row.monospaced
                            )
                        }
                    }
                }

                SurfaceCard(title: "操作") {
                    VStack(spacing: 12) {
                        ApprovalActionButton(
                            action: .allowTurn,
                            submittedAction: submittedAction,
                            isDisabled: actionButtonsDisabled,
                            tone: .primary
                        ) {
                            Task { await submit(.allowTurn) }
                        }

                        ApprovalActionButton(
                            action: .allowSession,
                            submittedAction: submittedAction,
                            isDisabled: actionButtonsDisabled,
                            tone: .info
                        ) {
                            Task { await submit(.allowSession) }
                        }

                        ApprovalActionButton(
                            action: .reject,
                            submittedAction: submittedAction,
                            isDisabled: actionButtonsDisabled,
                            tone: .critical
                        ) {
                            Task { await submit(.reject) }
                        }

                        if let feedback = feedbackMessage {
                            ApprovalFeedbackView(feedback: feedback)
                        }
                    }
                }
            }
            .padding()
        }
        .niumaScreenBackground()
        .navigationTitle("审批详情")
        .onChange(of: currentApproval.status) { _, status in
            if status == .resolved {
                submittedAction = nil
                localErrorMessage = nil
            }
        }
        .onChange(of: appModel.approvalFailureMessage(for: approval.approvalID)) { _, message in
            if message != nil {
                submittedAction = nil
            }
        }
    }

    private var actionButtonsDisabled: Bool {
        isResolved || isWaitingForConfirmation
    }

    private var feedbackMessage: ApprovalFeedback? {
        if let failureMessage {
            return ApprovalFeedback(text: failureMessage, tone: .critical)
        }
        if isResolved {
            return ApprovalFeedback(text: "审批已处理", tone: .positive)
        }
        if isWaitingForConfirmation {
            return ApprovalFeedback(text: "已提交，等待桌面确认", tone: .neutral)
        }
        return nil
    }

    private func submit(_ action: ApprovalResolutionAction) async {
        submittedAction = action
        localErrorMessage = nil
        do {
            switch action {
            case .allowTurn:
                try await appModel.resolveApproval(currentApproval, decision: .allow)
            case .allowSession:
                try await appModel.resolveApproval(
                    currentApproval,
                    decision: .allow,
                    grantScope: .session(
                        threadID: currentApproval.threadID,
                        approvalType: currentApproval.approvalType
                    )
                )
            case .reject:
                try await appModel.resolveApproval(currentApproval, decision: .reject)
            }
        } catch {
            submittedAction = nil
            localErrorMessage = error.localizedDescription
        }
    }
}

/// User-facing projection of a raw approval payload. It deliberately keeps
/// transport identifiers and callback method names out of the primary UI.
nonisolated struct ApprovalPresentation {
    struct ContextRow: Identifiable, Hashable {
        let id: String
        let title: String
        let value: String
        let monospaced: Bool
    }

    let typeTitle: String
    let typeSubtitle: String
    let iconName: String
    let actionText: String
    let actionMonospaced: Bool
    let reasonText: String?
    let contextRows: [ContextRow]

    init(approval: ApprovalSummary) {
        let params = Self.paramsDictionary(from: approval.paramsJSON)
        let type = Self.canonicalType(for: approval)
        var actionText = Self.fallbackActionText(for: approval)
        var actionMonospaced = false
        var contextRows: [ContextRow] = []

        switch type {
        case "shell_command":
            if let command = Self.cleanString(params["command"]) {
                actionText = command
                actionMonospaced = true
            } else if let firstAction = Self.commandActionSummaries(from: params).first {
                actionText = firstAction
            } else {
                actionText = "执行命令"
            }
            let actionSummaries = Self.commandActionSummaries(from: params)
            if !actionSummaries.isEmpty {
                contextRows.append(
                    ContextRow(
                        id: "command-actions",
                        title: "解析动作",
                        value: actionSummaries.prefix(3).joined(separator: "\n"),
                        monospaced: false
                    )
                )
            }
            if let cwd = Self.cleanString(params["cwd"]) {
                contextRows.append(ContextRow(id: "cwd", title: "工作目录", value: cwd, monospaced: true))
            }
        case "file_change":
            if let grantRoot = Self.cleanString(params["grantRoot"]) {
                actionText = "允许修改 \(grantRoot)"
                actionMonospaced = true
                contextRows.append(ContextRow(id: "grant-root", title: "影响范围", value: grantRoot, monospaced: true))
            } else {
                actionText = "允许本次文件变更"
            }
        case "permissions":
            actionText = "调整会话权限"
            if let permissionSummary = Self.permissionSummary(from: params["permissions"]) {
                contextRows.append(
                    ContextRow(
                        id: "permissions",
                        title: "权限范围",
                        value: permissionSummary,
                        monospaced: false
                    )
                )
            }
            if let cwd = Self.cleanString(params["cwd"]) {
                contextRows.append(ContextRow(id: "cwd", title: "工作目录", value: cwd, monospaced: true))
            }
        default:
            break
        }

        self.typeTitle = Self.typeTitle(for: type, fallback: approval.approvalType)
        self.typeSubtitle = Self.typeSubtitle(for: type)
        self.iconName = Self.iconName(for: type)
        self.actionText = actionText
        self.actionMonospaced = actionMonospaced
        self.reasonText = Self.cleanString(params["reason"])
        self.contextRows = contextRows
    }

    private static func canonicalType(for approval: ApprovalSummary) -> String {
        if approval.approvalType == "shell_command"
            || approval.requestMethod?.contains("commandExecution") == true {
            return "shell_command"
        }
        if approval.approvalType == "file_change"
            || approval.requestMethod?.contains("fileChange") == true {
            return "file_change"
        }
        if approval.approvalType == "permissions"
            || approval.requestMethod?.contains("permissions") == true {
            return "permissions"
        }
        return approval.approvalType
    }

    private static func typeTitle(for type: String, fallback: String) -> String {
        switch type {
        case "shell_command":
            return "命令执行"
        case "file_change":
            return "文件修改"
        case "permissions":
            return "权限申请"
        default:
            return fallback
        }
    }

    private static func typeSubtitle(for type: String) -> String {
        switch type {
        case "shell_command":
            return "需要确认这条命令是否可以执行"
        case "file_change":
            return "需要确认是否允许这次文件变更"
        case "permissions":
            return "需要确认是否调整当前会话权限"
        default:
            return "需要你确认这条请求是否可以继续"
        }
    }

    private static func iconName(for type: String) -> String {
        switch type {
        case "shell_command":
            return "terminal"
        case "file_change":
            return "doc.text"
        case "permissions":
            return "lock.shield"
        default:
            return "exclamationmark.bubble"
        }
    }

    private static func fallbackActionText(for approval: ApprovalSummary) -> String {
        if let requestMethod = approval.requestMethod, !requestMethod.isEmpty {
            return requestMethod
        }
        return approval.approvalType
    }

    private static func paramsDictionary(from json: String?) -> [String: Any] {
        guard let json,
              let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return [:]
        }
        return object
    }

    private static func cleanString(_ value: Any?) -> String? {
        guard let value = value as? String else { return nil }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    private static func commandActionSummaries(from params: [String: Any]) -> [String] {
        guard let actions = params["commandActions"] as? [[String: Any]] else { return [] }
        return actions.compactMap { action in
            let type = cleanString(action["type"])
            let command = cleanString(action["command"])
            switch type {
            case "read":
                if let name = cleanString(action["name"]) {
                    return "读取 \(name)"
                }
                if let path = cleanString(action["path"]) {
                    return "读取 \(path)"
                }
            case "listFiles":
                if let path = cleanString(action["path"]) {
                    return "列出 \(path)"
                }
                return "列出文件"
            case "search":
                if let query = cleanString(action["query"]) {
                    return "搜索 \(query)"
                }
                if let path = cleanString(action["path"]) {
                    return "搜索 \(path)"
                }
            default:
                break
            }
            return command
        }
    }

    private static func permissionSummary(from value: Any?) -> String? {
        guard let permissions = value as? [String: Any] else { return nil }
        var summaries: [String] = []

        if let fileSystem = permissions["fileSystem"] as? [String: Any] {
            if let entries = fileSystem["entries"] as? [[String: Any]] {
                summaries.append(contentsOf: entries.compactMap { entry in
                    permissionEntrySummary(entry)
                })
            }
            if let reads = stringArray(fileSystem["read"]), !reads.isEmpty {
                summaries.append("读取 \(reads.joined(separator: "、"))")
            }
            if let writes = stringArray(fileSystem["write"]), !writes.isEmpty {
                summaries.append("写入 \(writes.joined(separator: "、"))")
            }
        }

        if let network = permissions["network"] as? [String: Any],
           let enabled = network["enabled"] as? Bool {
            summaries.append(enabled ? "允许网络访问" : "关闭网络访问")
        }

        return summaries.isEmpty ? nil : summaries.prefix(4).joined(separator: "\n")
    }

    private static func permissionEntrySummary(_ entry: [String: Any]) -> String? {
        let accessTitle: String
        switch cleanString(entry["access"]) {
        case "read":
            accessTitle = "读取"
        case "write":
            accessTitle = "写入"
        case "none":
            accessTitle = "无访问"
        default:
            accessTitle = "访问"
        }

        guard let path = fileSystemPathDescription(entry["path"]) else {
            return nil
        }
        return "\(accessTitle) \(path)"
    }

    private static func fileSystemPathDescription(_ value: Any?) -> String? {
        if let string = cleanString(value) {
            return string
        }
        guard let object = value as? [String: Any] else { return nil }
        switch cleanString(object["type"]) {
        case "path":
            return cleanString(object["path"])
        case "glob_pattern":
            return cleanString(object["pattern"])
        case "special":
            return specialPathDescription(object["value"])
        default:
            return nil
        }
    }

    private static func specialPathDescription(_ value: Any?) -> String? {
        guard let object = value as? [String: Any] else { return nil }
        switch cleanString(object["kind"]) {
        case "root":
            return "全磁盘"
        case "minimal":
            return "默认最小范围"
        case "project_roots":
            if let subpath = cleanString(object["subpath"]) {
                return "项目目录/\(subpath)"
            }
            return "项目目录"
        case "tmpdir":
            return "临时目录"
        case "slash_tmp":
            return "/tmp"
        case "unknown":
            if let path = cleanString(object["path"]),
               let subpath = cleanString(object["subpath"]) {
                return "\(path)/\(subpath)"
            }
            return cleanString(object["path"])
        default:
            return nil
        }
    }

    private static func stringArray(_ value: Any?) -> [String]? {
        guard let values = value as? [Any] else { return nil }
        let strings = values.compactMap { value in
            cleanString(value)
        }
        return strings.isEmpty ? nil : strings
    }
}

private struct ApprovalTypeSummaryView: View {
    let presentation: ApprovalPresentation

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: presentation.iconName)
                .font(.system(size: 20, weight: .semibold))
                .foregroundStyle(NiumaPalette.warning)
                .frame(width: 42, height: 42)
                .background(Circle().fill(NiumaPalette.warningSoft))

            VStack(alignment: .leading, spacing: 4) {
                Text(presentation.typeTitle)
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(NiumaPalette.ink)
                Text(presentation.typeSubtitle)
                    .font(.footnote)
                    .foregroundStyle(NiumaPalette.mutedInk)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct ApprovalReadableRow: View {
    let title: String
    let value: String
    var monospaced = false

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(title)
                .font(.caption.weight(.semibold))
                .foregroundStyle(NiumaPalette.mutedInk)
            Text(value)
                .font(monospaced ? .footnote.monospaced() : .body)
                .foregroundStyle(NiumaPalette.ink)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, 4)
    }
}

private struct ApprovalFeedback {
    let text: String
    let tone: StatusBadge.Tone
}

private struct ApprovalFeedbackView: View {
    let feedback: ApprovalFeedback

    var body: some View {
        Text(feedback.text)
            .font(.footnote.weight(.semibold))
            .foregroundStyle(feedback.tone.foreground)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            .background(feedback.tone.background, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
}

private struct ApprovalActionButton: View {
    let action: ApprovalResolutionAction
    let submittedAction: ApprovalResolutionAction?
    let isDisabled: Bool
    let tone: Tone
    let perform: () -> Void

    private var isSubmitting: Bool {
        submittedAction == action
    }

    var body: some View {
        Button(action: perform) {
            HStack(spacing: 8) {
                if isSubmitting {
                    ProgressView()
                        .controlSize(.small)
                        .tint(tone.foreground)
                }
                Text(isSubmitting ? action.submittedTitle : action.title)
                    .lineLimit(1)
                    .minimumScaleFactor(0.85)
            }
        }
        .buttonStyle(ApprovalActionButtonStyle(tone: tone))
        .disabled(isDisabled)
        .opacity(isDisabled && !isSubmitting ? 0.56 : 1)
    }

    enum Tone {
        case primary
        case info
        case critical

        var foreground: Color {
            switch self {
            case .primary:
                return NiumaPalette.darkButtonText
            case .info:
                return NiumaPalette.info
            case .critical:
                return NiumaPalette.critical
            }
        }

        var background: Color {
            switch self {
            case .primary:
                return NiumaPalette.darkButton
            case .info:
                return NiumaPalette.infoSoft
            case .critical:
                return NiumaPalette.criticalSoft
            }
        }
    }
}

private struct ApprovalActionButtonStyle: ButtonStyle {
    let tone: ApprovalActionButton.Tone

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.headline)
            .foregroundStyle(tone.foreground)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 18)
            .padding(.vertical, 14)
            .background(tone.background, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            .opacity(configuration.isPressed ? 0.88 : 1)
            .scaleEffect(configuration.isPressed ? 0.99 : 1)
    }
}
