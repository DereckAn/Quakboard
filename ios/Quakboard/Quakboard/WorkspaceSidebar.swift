import SwiftUI

/// The iPad sidebar: the library, the clip types with counts, and the way to
/// Devices & Sync.
struct WorkspaceSidebar: View {
    @ObservedObject var model: SyncModel
    @Binding var selection: WorkspaceView.Section

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            AppTitle(subtitle: model.identity?.name ?? "iPad")
                .padding(.horizontal, 16)
                .padding(.vertical, 14)
            ScrollView {
                VStack(alignment: .leading, spacing: 4) {
                    groupLabel("Library")
                    row(.all, title: "All Clips", systemImage: "tray", count: model.page.counts.values.reduce(0, +))
                    row(.pinned, title: "Pinned", systemImage: "pin", count: model.page.pinnedCount)
                    groupLabel("Types")
                        .padding(.top, 12)
                    ForEach(HistoryItem.Kind.filterOrder, id: \.self) { kind in
                        row(.kind(kind), title: kind.pluralTitle, systemImage: kind.systemImage,
                            count: model.page.counts[kind] ?? 0)
                    }
                }
                .padding(.horizontal, 12)
            }
            syncRow
                .padding(12)
        }
        .background(Theme.sidebar)
    }

    private func groupLabel(_ title: String) -> some View {
        Text(title.uppercased())
            .font(.caption2.monospaced().weight(.semibold))
            .tracking(1)
            .foregroundStyle(Theme.textMuted)
            .padding(.horizontal, 12)
            .padding(.bottom, 4)
            .accessibilityAddTraits(.isHeader)
    }

    private func row(_ section: WorkspaceView.Section, title: String, systemImage: String, count: Int) -> some View {
        let isSelected = selection == section
        return Button { select(section) } label: {
            HStack(spacing: 12) {
                Image(systemName: systemImage)
                    .frame(width: 22)
                    .accessibilityHidden(true)
                Text(title)
                    .font(.body.weight(isSelected ? .semibold : .regular))
                Spacer()
                Text("\(count)")
                    .font(.caption2.monospaced().weight(.semibold))
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(isSelected ? Theme.onAccent.opacity(0.15) : Theme.inset, in: Capsule())
            }
            .foregroundStyle(isSelected ? Theme.onAccent : Theme.text)
            .padding(.horizontal, 12)
            .frame(height: 44)
            .background(isSelected ? Theme.accent : Color.clear, in: RoundedRectangle(cornerRadius: 12))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var syncRow: some View {
        let isSelected = selection == .settings
        return Button { select(.settings) } label: {
            HStack(spacing: 12) {
                Image(systemName: "laptopcomputer.and.iphone")
                    .frame(width: 22)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Devices & Sync")
                        .font(.subheadline.weight(.semibold))
                    Text(syncSummary)
                        .font(.caption)
                        .opacity(0.8)
                }
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold))
                    .accessibilityHidden(true)
            }
            .foregroundStyle(isSelected ? Theme.onAccent : Theme.text)
            .padding(12)
            .background(isSelected ? Theme.accent : Theme.card, in: RoundedRectangle(cornerRadius: 14))
            .overlay(RoundedRectangle(cornerRadius: 14).stroke(isSelected ? Color.clear : Theme.border))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var syncSummary: String {
        guard model.isSyncEnabled else { return "Sync is off" }
        guard !model.peers.isEmpty else { return "No paired devices" }
        let online = model.peers.filter(\.isOnline).count
        return "\(online) of \(model.peers.count) online"
    }

    private func select(_ section: WorkspaceView.Section) {
        selection = section
        switch section {
        case .kind(let kind): model.filter = kind
        case .all, .pinned: model.filter = nil
        case .settings: break
        }
    }
}
