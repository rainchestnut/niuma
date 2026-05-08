import SwiftUI

struct HomeView: View {
    @Environment(AppModel.self) private var appModel
    @State private var agentPendingRemoval: PairedAgent?

    var body: some View {
        Group {
            if appModel.hasPairedAgent {
                pairedHome
            } else {
                PairingView(dismissOnSuccess: false)
            }
        }
        .niumaScreenBackground()
        .toolbar(.hidden, for: .navigationBar)
        .confirmationDialog(
            appModel.localized("删除桌面绑定", "Remove Desktop Link"),
            isPresented: Binding(
                get: { agentPendingRemoval != nil },
                set: { newValue in
                    if !newValue {
                        agentPendingRemoval = nil
                    }
                }
            ),
            titleVisibility: .visible
        ) {
            if let agentPendingRemoval {
                Button(appModel.localized("删除绑定", "Remove Link"), role: .destructive) {
                    Task {
                        await appModel.removePairedAgent(agentPendingRemoval)
                        self.agentPendingRemoval = nil
                    }
                }
            }
            Button(appModel.localized("取消", "Cancel"), role: .cancel) {
                agentPendingRemoval = nil
            }
        } message: {
            if let agentPendingRemoval {
                Text(appModel.localized(
                    "将移除桌面 “\(agentPendingRemoval.displayName)” 的当前绑定。",
                    "This removes the current link to desktop “\(agentPendingRemoval.displayName)”."
                ))
            }
        }
    }

    private var pairedHome: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                homeHeader
                devicesSection
                projectsSection
                recentSection
            }
            .padding(.horizontal, 20)
            .padding(.top, 18)
            .padding(.bottom, 36)
        }
    }

    private var homeHeader: some View {
        HStack(alignment: .center, spacing: 16) {
            VStack(alignment: .leading, spacing: 4) {
                Text("Niuma")
                    .font(.system(size: 30, weight: .bold))
                    .foregroundStyle(NiumaPalette.ink)
                Text(appModel.localized("最近需要处理的项目和对话", "Projects and chats needing attention"))
                    .font(.caption)
                    .foregroundStyle(NiumaPalette.mutedInk)
            }

            Spacer()

            HStack(spacing: 0) {
                Button {
                    Task {
                        await appModel.refresh()
                    }
                } label: {
                    Group {
                        if appModel.isRefreshing {
                            ProgressView()
                                .tint(NiumaPalette.ink)
                        } else {
                            Image(systemName: "arrow.clockwise")
                                .font(.system(size: 15, weight: .semibold))
                        }
                    }
                            .frame(width: 34, height: 34)
                }
                .buttonStyle(.plain)

                Divider()
                    .frame(height: 16)
                    .overlay(NiumaPalette.border)
                    .padding(.horizontal, 2)

                NavigationLink {
                    SettingsView()
                } label: {
                    Image(systemName: "gearshape")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(NiumaPalette.ink)
                        .frame(width: 34, height: 34)
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 5)
            .background(
                Capsule(style: .continuous)
                    .fill(NiumaPalette.raisedCard)
            )
            .overlay(
                Capsule(style: .continuous)
                    .stroke(NiumaPalette.border, lineWidth: 1)
            )
        }
    }

    private var devicesSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            sectionHeader(appModel.localized("设备", "Devices"))

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 10) {
                    ForEach(appModel.pairedAgents) { agent in
                        Button {
                            Task {
                                await appModel.selectAgent(agent.agentID)
                            }
                        } label: {
                            DeviceChip(
                                agent: agent,
                                isSelected: appModel.selectedAgent?.agentID == agent.agentID
                            )
                        }
                        .buttonStyle(.plain)
                        .contextMenu {
                            Button(role: .destructive) {
                                agentPendingRemoval = agent
                            } label: {
                                Label(
                                    appModel.localized("删除绑定", "Remove Link"),
                                    systemImage: "trash"
                                )
                            }
                        }
                    }

                    NavigationLink {
                        PairingView()
                    } label: {
                        VStack(spacing: 6) {
                            Image(systemName: "plus")
                                .font(.system(size: 17, weight: .semibold))
                                .foregroundStyle(NiumaPalette.ink)
                                .frame(width: 38, height: 38)
                                .background(Circle().fill(NiumaPalette.raisedCard))
                            Text(appModel.localized("添加", "Add"))
                                .font(.caption)
                                .foregroundStyle(NiumaPalette.mutedInk)
                        }
                        .frame(width: 62)
                    }
                }
            }
        }
    }

    private var projectsSection: some View {
        VStack(alignment: .leading, spacing: 14) {
            sectionHeader(appModel.localized("项目", "Projects"))

            if appModel.visibleProjects.isEmpty {
                placeholderRow(appModel.localized("还没有同步到项目。", "No projects synced yet."))
            } else {
                VStack(spacing: 6) {
                    ForEach(appModel.visibleProjects) { project in
                        NavigationLink {
                            ProjectSessionsView(project: project)
                        } label: {
                            ProjectRow(project: project)
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("project-row")
                    }
                }
            }
        }
    }

    private var recentSection: some View {
        VStack(alignment: .leading, spacing: 14) {
            sectionHeader(appModel.localized("对话", "Chats")) {
                NavigationLink {
                    NewTaskView()
                } label: {
                    Image(systemName: "square.and.pencil")
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(NiumaPalette.ink)
                        .frame(width: 32, height: 32)
                        .background(Circle().fill(NiumaPalette.raisedCard))
                }
                .buttonStyle(.plain)
            }

            if appModel.conversationSessions.isEmpty {
                placeholderRow(appModel.localized("还没有可显示的对话。", "No chats available yet."))
            } else {
                VStack(spacing: 6) {
                    ForEach(appModel.conversationSessions) { session in
                        let sessionProject = appModel.project(for: session.projectID)
                        NavigationLink {
                            ThreadView(
                                project: sessionProject ?? ProjectSummary(
                                    projectID: session.projectID,
                                    agentID: session.agentID,
                                    projectName: appModel.localized("无项目", "No Project"),
                                    updatedAt: session.updatedAt
                                ),
                                session: session
                            )
                        } label: {
                            RecentSessionRow(
                                session: session,
                                pendingApprovals: appModel.pendingApprovalCount(for: session.threadID)
                            )
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("recent-session-row")
                    }
                }
            }
        }
    }

    private func sectionHeader<Trailing: View>(_ text: String, @ViewBuilder trailing: () -> Trailing = { EmptyView() }) -> some View {
        HStack(spacing: 12) {
            Text(text)
                .font(.system(size: 18, weight: .bold))
                .foregroundStyle(NiumaPalette.ink)
            Spacer()
            trailing()
        }
    }

    private func placeholderRow(_ text: String) -> some View {
        Text(text)
            .foregroundStyle(NiumaPalette.mutedInk)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, 18)
    }
}

private struct DeviceChip: View {
    @Environment(AppModel.self) private var appModel
    let agent: PairedAgent
    let isSelected: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Circle()
                .fill(isSelected ? NiumaPalette.info : NiumaPalette.neutralSoft)
                .frame(width: 28, height: 28)
                .overlay(
                    Image(systemName: "desktopcomputer")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(isSelected ? .white : NiumaPalette.ink)
                )

            Text(agent.displayName)
                .font(.caption2.weight(.semibold))
                .foregroundStyle(NiumaPalette.ink)
                .lineLimit(1)

            HStack(spacing: 6) {
                Circle()
                    .fill(agent.isOnline ? NiumaPalette.accent : NiumaPalette.mutedInk.opacity(0.35))
                    .frame(width: 7, height: 7)
                Text(appModel.localized(agent.isOnline ? "在线" : "离线", agent.isOnline ? "Online" : "Offline"))
                    .font(.caption2)
                    .foregroundStyle(NiumaPalette.mutedInk)
            }
        }
        .frame(width: 78, alignment: .leading)
        .padding(.horizontal, 10)
        .padding(.vertical, 9)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(isSelected ? NiumaPalette.infoSoft : NiumaPalette.raisedCard)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(isSelected ? NiumaPalette.info.opacity(0.22) : NiumaPalette.border, lineWidth: 1)
        )
    }
}

private struct ProjectRow: View {
    @Environment(AppModel.self) private var appModel
    let project: ProjectSummary

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: "folder")
                .font(.system(size: 21, weight: .medium))
                .foregroundStyle(NiumaPalette.ink)
                .frame(width: 34)

            VStack(alignment: .leading, spacing: 4) {
                Text(project.projectName)
                    .font(.system(size: 16, weight: .medium))
                    .foregroundStyle(NiumaPalette.ink)
                Text(DateFormatting.relativeTimestamp.localizedString(for: project.updatedAt, relativeTo: .now))
                    .font(.caption2)
                    .foregroundStyle(NiumaPalette.mutedInk)
            }

            Spacer()

            Image(systemName: "chevron.right")
                .font(.caption.weight(.bold))
                .foregroundStyle(NiumaPalette.mutedInk)
        }
        .padding(.vertical, 8)
        .contentShape(Rectangle())
    }
}

private struct RecentSessionRow: View {
    @Environment(AppModel.self) private var appModel
    let session: ThreadSummary
    let pendingApprovals: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 5) {
                    Text(session.title)
                        .font(.system(size: 15, weight: .medium))
                        .foregroundStyle(NiumaPalette.ink)
                        .multilineTextAlignment(.leading)
                    Text(DateFormatting.relativeTimestamp.localizedString(for: session.updatedAt, relativeTo: .now))
                        .font(.caption2)
                        .foregroundStyle(NiumaPalette.mutedInk)
                }

                Spacer()

                Image(systemName: "chevron.right")
                    .font(.caption.weight(.bold))
                    .foregroundStyle(NiumaPalette.mutedInk)
            }

            HStack(spacing: 8) {
                if pendingApprovals > 0 {
                    StatusBadge(
                        title: appModel.localized("\(pendingApprovals) 待审批", "\(pendingApprovals) approvals"),
                        tone: .warning
                    )
                }
                if let statusBadge = session.status.compactBadge(for: appModel.appLanguage) {
                    StatusBadge(title: statusBadge.0, tone: statusBadge.1)
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .background(
            RoundedRectangle(cornerRadius: 20, style: .continuous)
                .fill(NiumaPalette.raisedCard)
        )
        .contentShape(Rectangle())
    }
}
