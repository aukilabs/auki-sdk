import Foundation
import Security

enum KeychainSeedStoreError: Error, Equatable {
    case randomGenerationFailed(OSStatus)
    case keychainReadFailed(OSStatus)
    case keychainWriteFailed(OSStatus)
    case invalidSeedLength(Int)
}

struct KeychainSeedStore {
    let service: String
    let account: String

    init(
        service: String = "com.aukilabs.examples.AukiCameraStreamer",
        account: String = "wallet-seed"
    ) {
        self.service = service
        self.account = account
    }

    func loadOrCreateSeed() throws -> Data {
        if let seed = try loadSeed() {
            guard seed.count == 32 else {
                throw KeychainSeedStoreError.invalidSeedLength(seed.count)
            }
            return seed
        }

        let seed = try makeSeed()
        if try saveSeed(seed) {
            return seed
        }
        guard let storedSeed = try loadSeed() else {
            throw KeychainSeedStoreError.keychainReadFailed(errSecItemNotFound)
        }
        guard storedSeed.count == 32 else {
            throw KeychainSeedStoreError.invalidSeedLength(storedSeed.count)
        }
        return storedSeed
    }

    private func loadSeed() throws -> Data? {
        var query = baseQuery()
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne

        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess else {
            throw KeychainSeedStoreError.keychainReadFailed(status)
        }
        return result as? Data
    }

    private func saveSeed(_ seed: Data) throws -> Bool {
        var query = baseQuery()
        query[kSecValueData as String] = seed
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly

        let status = SecItemAdd(query as CFDictionary, nil)
        if status == errSecDuplicateItem {
            return false
        }
        guard status == errSecSuccess else {
            throw KeychainSeedStoreError.keychainWriteFailed(status)
        }
        return true
    }

    private func makeSeed() throws -> Data {
        var bytes = [UInt8](repeating: 0, count: 32)
        let status = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
        guard status == errSecSuccess else {
            throw KeychainSeedStoreError.randomGenerationFailed(status)
        }
        return Data(bytes)
    }

    private func baseQuery() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
    }
}
