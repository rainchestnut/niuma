import Foundation

enum AppContainer {
    /// Builds the production app model with live controller, identity services, and SwiftData store.
    @MainActor
    static func bootstrap() -> AppModel {
        let storage = UserDefaultsStore(defaults: .standard)
        let secretStore = KeychainSecretStore(service: "com.rainchestnut.niuma.device")
        let identityService = DeviceIdentityService(storage: storage, secretStore: secretStore)
        let dataStore = NiumaDataStore.live()
        let threadSyncPipeline = ThreadSyncPipeline.live()

        let serverBaseURL = AppModel.initialServerBaseURL(storage: storage)

        let appModel = AppModel(
            serverBaseURL: serverBaseURL,
            controllerFactory: { baseURL in
                LiveNiumaController(baseURL: baseURL, identityService: identityService)
            },
            identityService: identityService,
            storage: storage,
            dataStore: dataStore,
            threadSyncPipeline: threadSyncPipeline
        )
        return appModel
    }
}
