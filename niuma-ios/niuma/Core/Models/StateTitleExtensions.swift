import Foundation

extension DeviceLifecycleState {
    func title(for language: AppLanguage) -> String {
        switch (self, language) {
        case (.uninitialized, .chinese):
            return "未初始化"
        case (.uninitialized, .english):
            return "Uninitialized"
        case (.deviceRegistered, .chinese):
            return "已注册"
        case (.deviceRegistered, .english):
            return "Registered"
        case (.unpaired, .chinese):
            return "未配对"
        case (.unpaired, .english):
            return "Unpaired"
        case (.pairing, .chinese):
            return "配对中"
        case (.pairing, .english):
            return "Pairing"
        case (.paired, .chinese):
            return "已配对"
        case (.paired, .english):
            return "Paired"
        case (.pairFailed, .chinese):
            return "配对失败"
        case (.pairFailed, .english):
            return "Pair Failed"
        }
    }
}

extension RealtimeConnectionState {
    func title(for language: AppLanguage) -> String {
        switch (self, language) {
        case (.disconnected, .chinese):
            return "未连接"
        case (.disconnected, .english):
            return "Offline"
        case (.authenticating, .chinese):
            return "鉴权中"
        case (.authenticating, .english):
            return "Authenticating"
        case (.connected, .chinese):
            return "在线"
        case (.connected, .english):
            return "Connected"
        case (.degraded, .chinese):
            return "降级"
        case (.degraded, .english):
            return "Degraded"
        case (.retrying, .chinese):
            return "重试中"
        case (.retrying, .english):
            return "Retrying"
        }
    }
}

extension ThreadRuntimeState {
    func title(for language: AppLanguage) -> String {
        switch (self, language) {
        case (.idle, .chinese):
            return "空闲"
        case (.idle, .english):
            return "Idle"
        case (.submitting, .chinese):
            return "提交中"
        case (.submitting, .english):
            return "Submitting"
        case (.streaming, .chinese):
            return "流式中"
        case (.streaming, .english):
            return "Streaming"
        case (.waitingApproval, .chinese):
            return "待审批"
        case (.waitingApproval, .english):
            return "Awaiting Approval"
        case (.reconnecting, .chinese):
            return "重连中"
        case (.reconnecting, .english):
            return "Reconnecting"
        case (.completed, .chinese):
            return "已完成"
        case (.completed, .english):
            return "Completed"
        case (.failed, .chinese):
            return "失败"
        case (.failed, .english):
            return "Failed"
        }
    }
}

extension ThreadRefreshPhase {
    func title(for language: AppLanguage) -> String {
        L10n.string("refresh.phase.\(rawValue)", language: language)
    }
}

extension ThreadStatus {
    func title(for language: AppLanguage) -> String {
        switch (self, language) {
        case (.notLoaded, .chinese):
            return "空闲"
        case (.notLoaded, .english):
            return "Idle"
        case (.created, .chinese):
            return "已创建"
        case (.created, .english):
            return "Created"
        case (.idle, .chinese):
            return "空闲"
        case (.idle, .english):
            return "Idle"
        case (.running, .chinese):
            return "运行中"
        case (.running, .english):
            return "Running"
        case (.waitingApproval, .chinese):
            return "待审批"
        case (.waitingApproval, .english):
            return "Awaiting Approval"
        case (.pending, .chinese):
            return "待处理"
        case (.pending, .english):
            return "Pending"
        case (.threadStarted, .chinese):
            return "已启动"
        case (.threadStarted, .english):
            return "Thread Started"
        case (.completed, .chinese):
            return "已完成"
        case (.completed, .english):
            return "Completed"
        case (.failed, .chinese):
            return "失败"
        case (.failed, .english):
            return "Failed"
        case (.systemError, .chinese):
            return "系统错误"
        case (.systemError, .english):
            return "System Error"
        case (.cancelled, .chinese):
            return "已取消"
        case (.cancelled, .english):
            return "Cancelled"
        case (.closed, .chinese):
            return "已关闭"
        case (.closed, .english):
            return "Closed"
        case (.archived, .chinese):
            return "已归档"
        case (.archived, .english):
            return "Archived"
        case (.unknown, .chinese):
            return "未知"
        case (.unknown, .english):
            return "Unknown"
        }
    }
}
