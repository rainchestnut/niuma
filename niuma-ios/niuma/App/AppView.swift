import SwiftUI

struct AppView: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.scenePhase) private var scenePhase
    @State private var navigationPath = NavigationPath()

    var body: some View {
        NavigationStack(path: $navigationPath) {
            HomeView()
                .navigationDestination(for: PushThreadRoute.self) { route in
                    PushThreadDestination(route: route)
                }
        }
        .tint(NiumaPalette.accent)
        .environment(\.locale, Locale(identifier: appModel.localeIdentifier))
        .preferredColorScheme(appModel.preferredColorScheme)
        .task {
            PushNotificationCoordinator.shared.attach(appModel)
            await appModel.bootstrapIfNeeded()
        }
        .onChange(of: appModel.pendingPushThreadRoute) { _, route in
            guard let route else { return }
            navigationPath.append(route)
            appModel.pendingPushThreadRoute = nil
        }
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active else { return }
            Task {
                await appModel.resumeAfterActivation()
            }
        }
        .alert("出现问题", isPresented: Binding(
            get: { appModel.pendingError != nil },
            set: { newValue in
                if !newValue {
                    appModel.dismissError()
                }
            }
        )) {
            Button(appModel.localized("知道了", "OK"), role: .cancel) {
                appModel.dismissError()
            }
        } message: {
            Text(appModel.pendingError ?? "")
        }
    }
}

private struct PushThreadDestination: View {
    @Environment(AppModel.self) private var appModel

    let route: PushThreadRoute

    var body: some View {
        Group {
            if let thread = appModel.threadSummary(for: route.threadID) {
                ThreadView(
                    project: appModel.project(for: thread.projectID) ?? ProjectSummary(
                        projectID: thread.projectID,
                        agentID: thread.agentID,
                        projectName: appModel.localized("无项目", "No Project"),
                        updatedAt: thread.updatedAt
                    ),
                    session: thread
                )
                .task {
                    await appModel.refreshThreadDetails(threadID: thread.threadID)
                }
            } else {
                ContentUnavailableView(
                    appModel.localized("正在同步任务进度", "Syncing task progress"),
                    systemImage: "bell.badge",
                    description: Text(appModel.localized("正在从桌面端读取最新详情。", "Reading the latest details from the desktop."))
                )
                .task {
                    await appModel.refresh()
                    await appModel.refreshThreadDetails(threadID: route.threadID)
                }
            }
        }
    }
}
