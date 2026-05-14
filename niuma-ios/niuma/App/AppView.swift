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
            Task {
                switch phase {
                case .active:
                    await appModel.resumeAfterActivation()
                case .background:
                    await appModel.suspendRealtimeForBackground()
                case .inactive:
                    break
                @unknown default:
                    break
                }
            }
        }
        .alert(appModel.localized("app.error.title"), isPresented: Binding(
            get: { appModel.pendingError != nil },
            set: { newValue in
                if !newValue {
                    appModel.dismissError()
                }
            }
        )) {
            Button(appModel.localized("common.ok"), role: .cancel) {
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
                        projectName: appModel.localized("project.none"),
                        updatedAt: thread.updatedAt
                    ),
                    session: thread
                )
                .task {
                    await appModel.refreshThreadDetails(threadID: thread.threadID)
                }
            } else {
                ContentUnavailableView(
                    appModel.localized("push_thread.syncing.title"),
                    systemImage: "bell.badge",
                    description: Text(appModel.localized("push_thread.syncing.description"))
                )
                .task {
                    await appModel.refresh()
                    await appModel.refreshThreadDetails(threadID: route.threadID)
                }
            }
        }
    }
}
