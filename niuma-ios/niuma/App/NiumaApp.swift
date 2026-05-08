import SwiftUI

@main
struct NiumaApp: App {
    @UIApplicationDelegateAdaptor(NiumaAppDelegate.self) private var appDelegate
    @State private var appModel: AppModel

    init() {
        _appModel = State(initialValue: AppContainer.bootstrap())
    }

    var body: some Scene {
        WindowGroup {
            AppView()
                .environment(appModel)
        }
    }
}
