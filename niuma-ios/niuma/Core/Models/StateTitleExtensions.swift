import Foundation

extension DeviceLifecycleState {
    func title(for language: AppLanguage) -> String {
        L10n.string("device_state.\(rawValue)", language: language)
    }
}

extension RealtimeConnectionState {
    func title(for language: AppLanguage) -> String {
        L10n.string("realtime_state.\(rawValue)", language: language)
    }
}

extension ThreadRuntimeState {
    func title(for language: AppLanguage) -> String {
        L10n.string("runtime_state.\(rawValue)", language: language)
    }
}

extension ThreadRefreshPhase {
    func title(for language: AppLanguage) -> String {
        L10n.string("refresh.phase.\(rawValue)", language: language)
    }
}

extension ThreadStatus {
    func title(for language: AppLanguage) -> String {
        L10n.string("thread_status.\(rawValue)", language: language)
    }
}
