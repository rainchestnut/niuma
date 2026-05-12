import SwiftUI

struct UserInputPromptRow: View {
    @Environment(AppModel.self) private var appModel

    let request: UserInputRequestSummary

    private var activeRequest: UserInputRequestSummary {
        appModel.currentUserInputRequest(request.requestID) ?? request
    }

    var body: some View {
        HStack(alignment: .bottom, spacing: 0) {
            NavigationLink {
                UserInputPromptDetailView(request: request)
            } label: {
                VStack(alignment: .leading, spacing: 9) {
                    HStack(alignment: .top, spacing: 10) {
                        Image(systemName: "questionmark.bubble.fill")
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(NiumaPalette.warning)
                            .frame(width: 30, height: 30)
                            .background(Circle().fill(NiumaPalette.card.opacity(0.86)))

                        VStack(alignment: .leading, spacing: 4) {
                            HStack(spacing: 7) {
                                Text(copy("user_input.needs"))
                                    .font(.footnote.weight(.semibold))
                                    .foregroundStyle(NiumaPalette.ink)
                                Text(statusTitle(activeRequest.status))
                                    .font(.caption2.weight(.semibold))
                                    .foregroundStyle(statusTone(activeRequest.status).foreground)
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 4)
                                    .background(Capsule().fill(statusTone(activeRequest.status).background))
                            }

                            Text(primaryQuestionText)
                                .font(.footnote)
                                .foregroundStyle(NiumaPalette.ink)
                                .fixedSize(horizontal: false, vertical: true)

                            if let failure = appModel.userInputFailureMessage(for: request.requestID) {
                                Text(failure)
                                    .font(.caption)
                                    .foregroundStyle(NiumaPalette.critical)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }

                        Image(systemName: "chevron.right")
                            .font(.caption.weight(.bold))
                            .foregroundStyle(NiumaPalette.mutedInk)
                            .padding(.top, 6)
                    }

                    Text(DateFormatting.timeAndDate.string(from: request.updatedAt))
                        .font(.caption2)
                        .foregroundStyle(NiumaPalette.mutedInk)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .fill(NiumaPalette.warningSoft)
                )
            }
            .buttonStyle(UserInputPromptRowButtonStyle())
            .frame(maxWidth: .infinity, alignment: .leading)

            Spacer(minLength: 34)
        }
    }

    private var primaryQuestionText: String {
        request.questions.first?.prompt ?? copy("user_input.empty")
    }

    private func statusTitle(_ status: ApprovalStatus) -> String {
        switch status {
        case .pending:
            return copy("user_input.status.pending")
        case .submitting:
            return copy("user_input.status.submitting")
        case .resolved:
            return copy("user_input.status.resolved")
        case .failed:
            return copy("user_input.status.failed")
        }
    }

    private func statusTone(_ status: ApprovalStatus) -> StatusBadge.Tone {
        switch status {
        case .pending, .submitting:
            return .warning
        case .resolved:
            return .positive
        case .failed:
            return .critical
        }
    }

    private func copy(_ key: String) -> String {
        UserInputPromptCopy.text(key, language: appModel.appLanguage)
    }
}

struct UserInputPromptDetailView: View {
    @Environment(AppModel.self) private var appModel

    let request: UserInputRequestSummary

    @State private var selectedAnswers: [String: Set<String>]
    @State private var textAnswers: [String: String]
    @State private var localErrorMessage: String?
    @State private var isWaitingForConfirmation = false

    init(request: UserInputRequestSummary) {
        self.request = request
        _selectedAnswers = State(initialValue: Dictionary(uniqueKeysWithValues: request.questions.map { ($0.questionID, []) }))
        _textAnswers = State(initialValue: Dictionary(uniqueKeysWithValues: request.questions.map { ($0.questionID, "") }))
    }

    private var currentRequest: UserInputRequestSummary {
        appModel.currentUserInputRequest(request.requestID) ?? request
    }

    private var failureMessage: String? {
        localErrorMessage ?? appModel.userInputFailureMessage(for: request.requestID)
    }

    private var isResolved: Bool {
        currentRequest.status == .resolved
    }

    private var isSubmitting: Bool {
        currentRequest.status == .submitting || isWaitingForConfirmation
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                SurfaceCard(title: copy("user_input.detail.title"), subtitle: statusTitle(currentRequest.status)) {
                    VStack(alignment: .leading, spacing: 16) {
                        ForEach(currentRequest.questions, id: \.questionID) { question in
                            questionSection(question)
                        }
                    }
                    .disabled(isResolved || isSubmitting)
                    .opacity(isResolved || isSubmitting ? 0.68 : 1)
                }

                if let failureMessage {
                    Text(failureMessage)
                        .font(.footnote)
                        .foregroundStyle(NiumaPalette.critical)
                        .fixedSize(horizontal: false, vertical: true)
                }

                if let feedback = feedbackMessage {
                    Text(feedback.text)
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(feedback.tone.foreground)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(12)
                        .background(feedback.tone.background, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                }

                Button {
                    Task { await submit() }
                } label: {
                    HStack(spacing: 8) {
                        if isSubmitting {
                            ProgressView()
                                .controlSize(.small)
                        } else if isResolved {
                            Image(systemName: "checkmark.circle.fill")
                        } else {
                            Image(systemName: "paperplane.fill")
                        }
                        Text(submitButtonTitle)
                    }
                    .font(.headline)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 13)
                }
                .buttonStyle(.borderedProminent)
                .disabled(isSubmitting || isResolved)
            }
            .padding(18)
        }
        .niumaScreenBackground()
        .navigationTitle(copy("user_input.navigation_title"))
        .navigationBarTitleDisplayMode(.inline)
        .onChange(of: currentRequest.status) { _, status in
            if status == .resolved {
                isWaitingForConfirmation = false
                localErrorMessage = nil
            } else if status == .failed {
                isWaitingForConfirmation = false
            }
        }
    }

    @ViewBuilder
    private func questionSection(_ question: UserInputQuestion) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(question.header.isEmpty ? copy("user_input.question") : question.header)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(NiumaPalette.ink)
            Text(question.prompt)
                .font(.footnote)
                .foregroundStyle(NiumaPalette.mutedInk)
                .fixedSize(horizontal: false, vertical: true)

            if !question.options.isEmpty {
                VStack(spacing: 8) {
                    ForEach(question.options, id: \.label) { option in
                        optionButton(questionID: question.questionID, option: option)
                    }
                }
            }

            if question.options.isEmpty || question.isOther {
                Group {
                    if question.isSecret {
                        SecureField(copy("user_input.text_placeholder"), text: textBinding(for: question.questionID))
                    } else {
                        TextField(copy("user_input.text_placeholder"), text: textBinding(for: question.questionID), axis: .vertical)
                            .lineLimit(2...5)
                    }
                }
                .textFieldStyle(.roundedBorder)
            }
        }
    }

    private func optionButton(questionID: String, option: UserInputQuestionOption) -> some View {
        let isSelected = selectedAnswers[questionID, default: []].contains(option.label)
        return Button {
            toggleOption(questionID: questionID, label: option.label)
        } label: {
            HStack(spacing: 10) {
                Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(isSelected ? NiumaPalette.accent : NiumaPalette.mutedInk)
                VStack(alignment: .leading, spacing: 2) {
                    Text(option.label)
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(NiumaPalette.ink)
                    if !option.description.isEmpty {
                        Text(option.description)
                            .font(.caption)
                            .foregroundStyle(NiumaPalette.mutedInk)
                    }
                }
                Spacer()
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(NiumaPalette.card, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(isSelected ? NiumaPalette.accent.opacity(0.45) : NiumaPalette.border, lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
    }

    private func textBinding(for questionID: String) -> Binding<String> {
        Binding(
            get: { textAnswers[questionID, default: ""] },
            set: { textAnswers[questionID] = $0 }
        )
    }

    private func toggleOption(questionID: String, label: String) {
        var answers = selectedAnswers[questionID, default: []]
        if answers.contains(label) {
            answers.remove(label)
        } else {
            answers.insert(label)
        }
        selectedAnswers[questionID] = answers
    }

    private func submit() async {
        let answers = currentRequest.questions.reduce(into: [String: [String]]()) { result, question in
            var values = Array(selectedAnswers[question.questionID, default: []]).sorted()
            let text = textAnswers[question.questionID, default: ""].trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty {
                values.append(text)
            }
            result[question.questionID] = values
        }
        guard answers.values.allSatisfy({ !$0.isEmpty }) else {
            localErrorMessage = copy("user_input.validation")
            return
        }
        localErrorMessage = nil
        isWaitingForConfirmation = true
        do {
            try await appModel.respondToUserInput(currentRequest, answers: answers)
            if appModel.currentUserInputRequest(request.requestID)?.status == .resolved {
                isWaitingForConfirmation = false
            }
        } catch {
            isWaitingForConfirmation = false
            localErrorMessage = error.localizedDescription
        }
    }

    private var feedbackMessage: (text: String, tone: StatusBadge.Tone)? {
        if failureMessage != nil {
            return nil
        }
        if isResolved {
            return (copy("user_input.feedback.resolved"), .positive)
        }
        if isSubmitting {
            return (copy("user_input.feedback.waiting"), .neutral)
        }
        return nil
    }

    private var submitButtonTitle: String {
        if isResolved {
            return copy("user_input.submit.done")
        }
        if isSubmitting {
            return copy("user_input.submit.waiting")
        }
        return copy("user_input.submit")
    }

    private func statusTitle(_ status: ApprovalStatus) -> String {
        UserInputPromptCopy.text("user_input.status.\(status.rawValue)", language: appModel.appLanguage)
    }

    private func copy(_ key: String) -> String {
        UserInputPromptCopy.text(key, language: appModel.appLanguage)
    }
}

enum UserInputPromptCopy {
    static func text(_ key: String, language: AppLanguage) -> String {
        switch (key, language) {
        case ("user_input.needs", .english): return "Input needed"
        case ("user_input.needs", .chinese): return "需要输入"
        case ("user_input.empty", .english): return "Codex is waiting for more information."
        case ("user_input.empty", .chinese): return "Codex 正在等待补充信息。"
        case ("user_input.status.pending", .english): return "Pending"
        case ("user_input.status.pending", .chinese): return "待处理"
        case ("user_input.status.submitting", .english): return "Submitting"
        case ("user_input.status.submitting", .chinese): return "提交中"
        case ("user_input.status.resolved", .english): return "Resolved"
        case ("user_input.status.resolved", .chinese): return "已完成"
        case ("user_input.status.failed", .english): return "Failed"
        case ("user_input.status.failed", .chinese): return "失败"
        case ("user_input.detail.title", .english): return "Requested input"
        case ("user_input.detail.title", .chinese): return "请求输入"
        case ("user_input.navigation_title", .english): return "Input"
        case ("user_input.navigation_title", .chinese): return "输入"
        case ("user_input.question", .english): return "Question"
        case ("user_input.question", .chinese): return "问题"
        case ("user_input.text_placeholder", .english): return "Type an answer"
        case ("user_input.text_placeholder", .chinese): return "输入回答"
        case ("user_input.submit", .english): return "Submit"
        case ("user_input.submit", .chinese): return "提交"
        case ("user_input.submit.waiting", .english): return "Sent"
        case ("user_input.submit.waiting", .chinese): return "已发送"
        case ("user_input.submit.done", .english): return "Completed"
        case ("user_input.submit.done", .chinese): return "已完成"
        case ("user_input.feedback.waiting", .english): return "Sent. Waiting for desktop confirmation."
        case ("user_input.feedback.waiting", .chinese): return "已发送，等待桌面确认。"
        case ("user_input.feedback.resolved", .english): return "Input completed."
        case ("user_input.feedback.resolved", .chinese): return "输入已完成。"
        case ("user_input.validation", .english): return "Answer every question before submitting."
        case ("user_input.validation", .chinese): return "请先回答所有问题。"
        default: return key
        }
    }
}

private struct UserInputPromptRowButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
            .opacity(configuration.isPressed ? 0.86 : 1)
            .scaleEffect(configuration.isPressed ? 0.99 : 1)
    }
}
