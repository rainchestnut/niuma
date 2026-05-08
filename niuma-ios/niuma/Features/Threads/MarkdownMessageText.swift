import Foundation
import SwiftUI

struct MarkdownMessageText: View {
    let blocks: [MarkdownBlock]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(blocks) { block in
                MarkdownBlockView(block: block)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct MarkdownBlockView: View {
    let block: MarkdownBlock

    var body: some View {
        switch block.kind {
        case .heading(let level, let text):
            MarkdownInlineText(text: text)
                .font(level == 1 ? .subheadline.weight(.bold) : .caption.weight(.semibold))
        case .paragraph(let text):
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(text.components(separatedBy: "\n").enumerated()), id: \.offset) { _, line in
                    MarkdownInlineText(text: line)
                        .font(.caption)
                }
            }
        case .bullet(let text):
            HStack(alignment: .top, spacing: 8) {
                Text("\u{2022}")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(NiumaPalette.ink)
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(text.components(separatedBy: "\n").enumerated()), id: \.offset) { _, line in
                        MarkdownInlineText(text: line)
                            .font(.caption)
                    }
                }
            }
        case .numbered(let marker, let text):
            HStack(alignment: .top, spacing: 8) {
                Text(marker)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(NiumaPalette.ink)
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(text.components(separatedBy: "\n").enumerated()), id: \.offset) { _, line in
                        MarkdownInlineText(text: line)
                            .font(.caption)
                    }
                }
            }
        case .code(let text):
            ScrollView(.horizontal, showsIndicators: false) {
                Text(text)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(NiumaPalette.ink)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(NiumaPalette.canvas.opacity(0.55))
            )
        }
    }
}

private struct MarkdownInlineText: View {
    let text: String

    var body: some View {
        Group {
            if let attributed = try? AttributedString(markdown: text) {
                Text(attributed)
            } else {
                Text(text)
            }
        }
        .foregroundStyle(NiumaPalette.ink)
        .textSelection(.enabled)
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct MarkdownBlock: Identifiable, Equatable {
    enum Kind: Equatable {
        case heading(level: Int, text: String)
        case paragraph(text: String)
        case bullet(text: String)
        case numbered(marker: String, text: String)
        case code(text: String)
    }

    let id: String
    let kind: Kind

    /// Parses lightweight Markdown blocks for message rendering.
    static func parse(_ raw: String, idPrefix: String) -> [MarkdownBlock] {
        var blocks: [MarkdownBlock] = []
        var paragraphLines: [String] = []
        var codeFenceLines: [String] = []
        var isInsideCodeFence = false
        var nextBlockIndex = 0

        /// Appends one parsed block with a short, caller-provided stable id.
        func append(_ kind: Kind) {
            blocks.append(
                MarkdownBlock(
                    id: "\(idPrefix)-block-\(nextBlockIndex)",
                    kind: kind
                )
            )
            nextBlockIndex += 1
        }

        /// Replaces the latest block while preserving its render identity.
        func replaceLast(with kind: Kind) {
            guard let last = blocks.last else { return }
            blocks[blocks.count - 1] = MarkdownBlock(id: last.id, kind: kind)
        }

        /// Flushes accumulated Markdown paragraph text into a block.
        func flushParagraph() {
            let text = paragraphLines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty {
                append(.paragraph(text: text))
            }
            paragraphLines.removeAll(keepingCapacity: true)
        }

        /// Flushes an accumulated Markdown code fence into a block.
        func flushCodeFence() {
            let text = codeFenceLines.joined(separator: "\n").trimmingCharacters(in: .newlines)
            if !text.isEmpty {
                append(.code(text: text))
            }
            codeFenceLines.removeAll(keepingCapacity: true)
        }

        for rawLine in raw.components(separatedBy: .newlines) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("```") {
                if isInsideCodeFence {
                    flushCodeFence()
                } else {
                    flushParagraph()
                }
                isInsideCodeFence.toggle()
                continue
            }
            if isInsideCodeFence {
                codeFenceLines.append(rawLine)
                continue
            }
            if line.isEmpty {
                flushParagraph()
                continue
            }
            if rawLine.hasPrefix("  ") || rawLine.hasPrefix("\t") {
                if let last = blocks.last {
                    switch last.kind {
                    case .bullet(let text):
                        replaceLast(with: .bullet(text: text + "\n" + line))
                        continue
                    case .numbered(let marker, let text):
                        replaceLast(with: .numbered(marker: marker, text: text + "\n" + line))
                        continue
                    default:
                        break
                    }
                }
            }
            if let heading = parseHeading(line) {
                flushParagraph()
                append(heading)
                continue
            }
            if let bullet = parseBullet(line) {
                flushParagraph()
                append(bullet)
                continue
            }
            if let numbered = parseNumbered(line) {
                flushParagraph()
                append(numbered)
                continue
            }
            paragraphLines.append(rawLine)
        }

        if isInsideCodeFence {
            flushCodeFence()
        }
        flushParagraph()
        return blocks
    }

    private static func parseHeading(_ line: String) -> Kind? {
        let prefixCount = line.prefix { $0 == "#" }.count
        guard prefixCount > 0, prefixCount <= 6 else { return nil }
        let text = line.dropFirst(prefixCount).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return nil }
        return .heading(level: prefixCount, text: text)
    }

    private static func parseBullet(_ line: String) -> Kind? {
        guard line.hasPrefix("- ") || line.hasPrefix("* ") || line.hasPrefix("+ ") else { return nil }
        let text = String(line.dropFirst(2)).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return nil }
        return .bullet(text: text)
    }

    private static func parseNumbered(_ line: String) -> Kind? {
        let pattern = #"^(\d+\.)\s+(.+)$"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return nil }
        let range = NSRange(line.startIndex..<line.endIndex, in: line)
        guard let match = regex.firstMatch(in: line, range: range),
              let markerRange = Range(match.range(at: 1), in: line),
              let textRange = Range(match.range(at: 2), in: line) else {
            return nil
        }
        return .numbered(
            marker: String(line[markerRange]),
            text: String(line[textRange]).trimmingCharacters(in: .whitespacesAndNewlines)
        )
    }
}
