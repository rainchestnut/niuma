import SwiftUI

struct NewTaskView: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss

    @State private var selectedProjectID = ""
    @State private var prompt = ""
    @State private var isSubmitting = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                SurfaceCard(title: "项目", subtitle: "先选择要把任务发往哪个 desktop project。") {
                    Picker("目标项目", selection: $selectedProjectID) {
                        ForEach(appModel.workspaceProjects) { project in
                            Text(project.projectName).tag(project.projectID)
                        }
                    }
                    .pickerStyle(.menu)
                    .niumaInputChrome()
                }

                SurfaceCard(title: "任务", subtitle: "直接输入要发给桌面 Codex 的任务内容。") {
                    TextField("输入要发给桌面 Codex 的任务", text: $prompt, axis: .vertical)
                        .lineLimit(4...10)
                        .niumaInputChrome()
                }

                SurfaceCard(title: "提交") {
                    Button {
                        Task {
                            await submit()
                        }
                    } label: {
                        if isSubmitting {
                            ProgressView()
                                .tint(.white)
                                .frame(maxWidth: .infinity)
                        } else {
                            Text("发起任务")
                                .frame(maxWidth: .infinity)
                        }
                    }
                    .buttonStyle(NiumaPrimaryButtonStyle())
                    .disabled(isSubmitting || selectedProjectID.isEmpty)
                }
            }
            .padding()
        }
        .niumaScreenBackground()
        .navigationTitle("新任务")
        .task {
            if selectedProjectID.isEmpty {
                selectedProjectID = appModel.selectedProjectID ?? ""
            }
        }
    }

    private func submit() async {
        isSubmitting = true
        defer { isSubmitting = false }

        do {
            try await appModel.startNewTask(
                projectID: selectedProjectID,
                prompt: prompt
            )
            dismiss()
        } catch {
            appModel.pendingError = error.localizedDescription
        }
    }
}
