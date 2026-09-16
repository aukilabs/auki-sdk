import ExpoModulesCore
#if canImport(auki_sdk_swiftFFI)
import auki_sdk_swiftFFI
#endif

public class AukiSdkExpoModule: Module {
  // UniFFI Swift (ios/AukiSDK) is compiled into this pod; FFI comes from
  // Frameworks/AukiSDK.xcframework. canImport(auki_sdk_swiftFFI) gates the API.
  #if canImport(auki_sdk_swiftFFI)
  private let sessions = ExpoSessionRegistry()
  private let domainData = ExpoDomainDataRegistry()
  private var peers: [String: AukiPeer] = [:]
  private var identities: [String: AukiPeerIdentity] = [:]
  private var streams: [String: AukiStreamSubscription] = [:]
  private var messageSenders: [String: AukiMessageSender] = [:]
  private var urdfModels: [String: AukiUrdfModel] = [:]
  #endif

  public func definition() -> ModuleDefinition {
    Name("AukiSdkExpo")
    Events("onZitadelSaveRequested")

    AsyncFunction("_importZitadel") { (credentialsJson: String, environmentJson: String?) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let id = self.newId("session")
      let store = ExpoZitadelStore { [weak self] requestId in
        self?.sendEvent("onZitadelSaveRequested", ["sessionId": id, "requestId": requestId])
      }
      let session = try importZitadel(credentialsJson: credentialsJson, environmentJson: environmentJson, store: store)
      self.sessions.insert(id: id, session: session, store: store)
      return id
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("_zitadelCredentials") { (sessionId: String, requestId: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      return try await self.sessions.store(sessionId).credentialsJson(requestId: requestId)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("_ackZitadelSave") { (sessionId: String, requestId: String, success: Bool) -> Bool in
      #if canImport(auki_sdk_swiftFFI)
      return try await self.sessions.store(sessionId).acknowledge(requestId: requestId, success: success)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("_closeSession") { (sessionId: String) in
      #if canImport(auki_sdk_swiftFFI)
      await self.sessions.close(sessionId)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("loginDev") { (email: String, password: String, clientId: String?) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let session = try await withAuthErrors {
        try await AukiSession.loginDev(email: email, password: password, clientId: clientId)
      }
      let id = self.newId("session")
      self.sessions.insert(id: id, session: session)
      return id
      #else
      throw unsupported("AukiSDK XCFramework missing; run scripts/sync-ios-xcframework.sh")
      #endif
    }

    AsyncFunction("loginWithEnvironment") {
      (
        apiBaseUrl: String,
        ddsBaseUrl: String,
        dmsBaseUrl: String,
        email: String,
        password: String,
        clientId: String?
      ) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let session = try await withAuthErrors {
        try await AukiSession.loginWithEnvironment(
          apiBaseUrl: apiBaseUrl,
          ddsBaseUrl: ddsBaseUrl,
          dmsBaseUrl: dmsBaseUrl,
          email: email,
          password: password,
          clientId: clientId
        )
      }
      let id = self.newId("session")
      self.sessions.insert(id: id, session: session)
      return id
      #else
      throw unsupported("AukiSDK XCFramework missing; run scripts/sync-ios-xcframework.sh")
      #endif
    }

    AsyncFunction("accessibleDomains") { (sessionId: String) -> [[String: Any?]] in
      #if canImport(auki_sdk_swiftFFI)
      let session = try self.sessions.session(sessionId)
      let domains = try await withAuthErrors { try await session.accessibleDomains() }
      return domains.map { domain in
        [
          "id": domain.id,
          "name": domain.name,
          "description": domain.description,
          "organizationId": domain.organizationId,
        ]
      }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainsList") {
      (sessionId: String, queryJson: String, operationId: String) -> [String: Any] in
      #if canImport(auki_sdk_swiftFFI)
      let cancellation = self.domainData.beginOperation(operationId)
      defer { self.domainData.finishOperation(operationId) }
      let query = try Self.domainListQuery(queryJson)
      let page = try await withDataErrors {
        try await self.sessions.session(sessionId).domains().list(
          query: query,
          cancellation: cancellation
        )
      }
      return Self.mapDomainPage(page)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainsForPortal") {
      (
        sessionId: String,
        portal: String,
        organization: String?,
        operationId: String
      ) -> [[String: Any]] in
      #if canImport(auki_sdk_swiftFFI)
      let cancellation = self.domainData.beginOperation(operationId)
      defer { self.domainData.finishOperation(operationId) }
      let values = try await withDataErrors {
        try await self.sessions.session(sessionId).domains().forPortal(
          portal: portal,
          organization: organization,
          cancellation: cancellation
        )
      }
      return values.map(Self.mapPortalDomain)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainsPortals") {
      (sessionId: String, domainId: String, operationId: String) -> [[String: Any]] in
      #if canImport(auki_sdk_swiftFFI)
      let cancellation = self.domainData.beginOperation(operationId)
      defer { self.domainData.finishOperation(operationId) }
      let values = try await withDataErrors {
        try await self.sessions.session(sessionId).domains().portals(
          domainId: domainId,
          cancellation: cancellation
        )
      }
      return values.map(Self.mapPortal)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainsPortal") {
      (
        sessionId: String,
        domainId: String,
        portal: String,
        operationId: String
      ) -> [String: Any] in
      #if canImport(auki_sdk_swiftFFI)
      let cancellation = self.domainData.beginOperation(operationId)
      defer { self.domainData.finishOperation(operationId) }
      let value = try await withDataErrors {
        try await self.sessions.session(sessionId).domains().portal(
          domainId: domainId,
          portal: portal,
          cancellation: cancellation
        )
      }
      return Self.mapPortal(value)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainDataOpen") { (sessionId: String, domainId: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let client = try await withDataErrors {
        try self.sessions.session(sessionId).data(domainId: domainId)
      }
      let id = self.newId("data")
      self.domainData.insertClient(client, id: id)
      return id
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainDataList") {
      (clientId: String, queryJson: String, operationId: String) -> [[String: Any]] in
      #if canImport(auki_sdk_swiftFFI)
      let cancellation = self.domainData.beginOperation(operationId)
      defer { self.domainData.finishOperation(operationId) }
      let query = try Self.dataListQuery(queryJson)
      let values = try await withDataErrors {
        try await self.domainData.client(clientId).list(
          query: query,
          cancellation: cancellation
        )
      }
      return values.map(Self.mapDataMetadata)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainDataGet") {
      (clientId: String, dataId: String, operationId: String) -> [String: Any] in
      #if canImport(auki_sdk_swiftFFI)
      let cancellation = self.domainData.beginOperation(operationId)
      defer { self.domainData.finishOperation(operationId) }
      let value = try await withDataErrors {
        try await self.domainData.client(clientId).get(
          dataId: dataId,
          cancellation: cancellation
        )
      }
      return Self.mapDataMetadata(value)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainDataRead") {
      (clientId: String, dataId: String, operationId: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let cancellation = self.domainData.beginOperation(operationId)
      defer { self.domainData.finishOperation(operationId) }
      let bytes = try await withDataErrors {
        try await self.domainData.client(clientId).read(
          dataId: dataId,
          cancellation: cancellation
        )
      }
      return bytes.base64EncodedString()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainDataWrite") {
      (
        clientId: String,
        targetJson: String,
        bytesBase64: String,
        operationId: String
      ) -> [String: Any] in
      #if canImport(auki_sdk_swiftFFI)
      guard let bytes = Data(base64Encoded: bytesBase64) else {
        throw ExpoDataFailure(kind: "input", status: nil, message: "Data bytes are not base64")
      }
      let cancellation = self.domainData.beginOperation(operationId)
      defer { self.domainData.finishOperation(operationId) }
      let target = try Self.dataWriteTarget(targetJson)
      let value = try await withDataErrors {
        try await self.domainData.client(clientId).write(
          target: target,
          bytes: bytes,
          cancellation: cancellation
        )
      }
      return Self.mapDataMetadata(value)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainDataDelete") {
      (clientId: String, dataId: String, operationId: String) in
      #if canImport(auki_sdk_swiftFFI)
      let cancellation = self.domainData.beginOperation(operationId)
      defer { self.domainData.finishOperation(operationId) }
      try await withDataErrors {
        try await self.domainData.client(clientId).delete(
          dataId: dataId,
          cancellation: cancellation
        )
      }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainDataPoses") {
      (clientId: String, operationId: String) -> [[String: Any]] in
      #if canImport(auki_sdk_swiftFFI)
      let cancellation = self.domainData.beginOperation(operationId)
      defer { self.domainData.finishOperation(operationId) }
      let values = try await withDataErrors {
        try await self.domainData.client(clientId).poses(cancellation: cancellation)
      }
      return values.map(Self.mapPortalPose)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainDataPose") {
      (clientId: String, portal: String, operationId: String) -> [String: Any] in
      #if canImport(auki_sdk_swiftFFI)
      let cancellation = self.domainData.beginOperation(operationId)
      defer { self.domainData.finishOperation(operationId) }
      let value = try await withDataErrors {
        try await self.domainData.client(clientId).pose(
          portal: portal,
          cancellation: cancellation
        )
      }
      return Self.mapPortalPose(value)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainDataClose") { (clientId: String) in
      #if canImport(auki_sdk_swiftFFI)
      guard let client = self.domainData.removeClient(clientId) else { return }
      try await withDataErrors { try await client.close() }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("dataOperationCancel") { (operationId: String) in
      #if canImport(auki_sdk_swiftFFI)
      self.domainData.cancelOperation(operationId)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("dataDownloadStart") {
      (
        clientId: String,
        dataId: String,
        optionsJson: String,
        operationId: String
      ) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let cancellation = self.domainData.beginOperation(operationId)
      do {
        let transfer = try await withDataErrors {
          try await self.domainData.client(clientId).startDownload(
            dataId: dataId,
            options: try Self.transferOptions(optionsJson),
            cancellation: cancellation
          )
        }
        let id = self.newId("download")
        self.domainData.insertDownload(transfer, id: id, operationId: operationId)
        return id
      } catch {
        self.domainData.finishOperation(operationId)
        throw error
      }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("dataDownloadNext") { (downloadId: String) -> String? in
      #if canImport(auki_sdk_swiftFFI)
      let bytes = try await withDataErrors {
        try await self.domainData.download(downloadId).next()
      }
      return bytes?.base64EncodedString()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("dataDownloadCancel") { (downloadId: String) in
      #if canImport(auki_sdk_swiftFFI)
      try await withDataErrors { try self.domainData.download(downloadId).cancel() }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("dataDownloadClose") { (downloadId: String) in
      #if canImport(auki_sdk_swiftFFI)
      guard let transfer = self.domainData.removeDownload(downloadId) else { return }
      try await withDataErrors { try await transfer.close() }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("dataUploadStart") {
      (
        clientId: String,
        targetJson: String,
        size: Double,
        optionsJson: String,
        operationId: String
      ) -> String in
      #if canImport(auki_sdk_swiftFFI)
      guard size.isFinite, size.rounded(.towardZero) == size,
        size >= 1, size <= 9_007_199_254_740_991
      else {
        throw ExpoDataFailure(kind: "input", status: nil, message: "size must be a positive safe integer")
      }
      let cancellation = self.domainData.beginOperation(operationId)
      do {
        let transfer = try await withDataErrors {
          try await self.domainData.client(clientId).startUpload(
            target: try Self.dataWriteTarget(targetJson),
            size: UInt64(size),
            options: try Self.transferOptions(optionsJson),
            cancellation: cancellation
          )
        }
        let id = self.newId("upload")
        self.domainData.insertUpload(transfer, id: id, operationId: operationId)
        return id
      } catch {
        self.domainData.finishOperation(operationId)
        throw error
      }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("dataUploadNextMaximum") { (uploadId: String) -> Double? in
      #if canImport(auki_sdk_swiftFFI)
      return try await withDataErrors {
        try await self.domainData.upload(uploadId).nextMaximum().map { Double($0) }
      }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("dataUploadPush") { (uploadId: String, bytesBase64: String) in
      #if canImport(auki_sdk_swiftFFI)
      guard let bytes = Data(base64Encoded: bytesBase64) else {
        throw ExpoDataFailure(kind: "input", status: nil, message: "Upload chunk is not base64")
      }
      try await withDataErrors { try await self.domainData.upload(uploadId).push(bytes: bytes) }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("dataUploadResult") { (uploadId: String) -> [String: Any] in
      #if canImport(auki_sdk_swiftFFI)
      let value = try await withDataErrors {
        try await self.domainData.upload(uploadId).result()
      }
      return Self.mapDataMetadata(value)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("dataUploadCancel") { (uploadId: String) in
      #if canImport(auki_sdk_swiftFFI)
      try await withDataErrors { try self.domainData.upload(uploadId).cancel() }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("dataUploadClose") { (uploadId: String) in
      #if canImport(auki_sdk_swiftFFI)
      guard let transfer = self.domainData.removeUpload(uploadId) else { return }
      try await withDataErrors { try await transfer.close() }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("startPeer") { (sessionId: String, domainId: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      return try await withAuthErrors { try await self.startPeer(sessionId: sessionId, domainId: domainId, mode: nil) }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("startPeerWithDiscovery") {
      (sessionId: String, domainId: String, mode: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let discovery: AukiDiscoveryMode =
        mode == "DiscoverAndAdvertise" ? .discoverAndAdvertise : .discoverOnly
      return try await withAuthErrors { try await self.startPeer(sessionId: sessionId, domainId: domainId, mode: discovery) }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("peerId") { (peerHandle: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      return try self.requirePeer(peerHandle).peerId()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainId") { (peerHandle: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      return try self.requirePeer(peerHandle).domainId()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("discover") { (peerHandle: String) -> [[String: Any]] in
      #if canImport(auki_sdk_swiftFFI)
      let candidates = try await self.requirePeer(peerHandle).discover()
      return candidates.map(Self.mapCandidate)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("discoverProtocol") {
      (peerHandle: String, protocolId: String) -> [[String: Any]] in
      #if canImport(auki_sdk_swiftFFI)
      let candidates = try await self.requirePeer(peerHandle).discoverProtocol(
        protocolId: protocolId
      )
      return candidates.map(Self.mapCandidate)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("infoFetchExact") {
      (peerHandle: String, target: [String: String]) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let peer = try self.requirePeer(peerHandle)
      let exact = try Self.exactTarget(target, domainId: peer.domainId())
      let json = try await AukiInfoClient(peer: peer).fetchExact(target: exact)
      return json
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("catalogFetchResourcesExact") {
      (peerHandle: String, target: [String: String], variants: [String]) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let peer = try self.requirePeer(peerHandle)
      let exact = try Self.exactTarget(target, domainId: peer.domainId())
      let json = try await AukiCatalogClient(peer: peer).fetchResourcesExact(
        target: exact,
        variants: []
      )
      _ = variants
      return json
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("registryListExact") {
      (peerHandle: String, target: [String: String], kind: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let peer = try self.requirePeer(peerHandle)
      let exact = try Self.exactTarget(target, domainId: peer.domainId())
      let registryKind = try Self.registryKind(kind)
      let entries = try await AukiRegistryClient(peer: peer).listExact(
        target: exact,
        kind: registryKind
      )
      let mapped: [[String: String]] = entries.map { entry in
        ["id": entry.id, "hash": entry.hash]
      }
      return try Self.jsonString(mapped)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("registryFetchExact") {
      (
        peerHandle: String,
        target: [String: String],
        kind: String,
        id: String,
        hash: String
      ) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let peer = try self.requirePeer(peerHandle)
      let exact = try Self.exactTarget(target, domainId: peer.domainId())
      let registryKind = try Self.registryKind(kind)
      return try await AukiRegistryClient(peer: peer).fetchExact(
        target: exact,
        kind: registryKind,
        id: id,
        hash: hash
      )
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("blobFetchExact") {
      (peerHandle: String, target: [String: String], sha256: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let peer = try self.requirePeer(peerHandle)
      let exact = try Self.exactTarget(target, domainId: peer.domainId())
      let receipt = try await AukiBlobClient(peer: peer).fetchExact(
        target: exact,
        sha256: sha256
      )
      return try Self.jsonString([
        "peerId": receipt.remotePeerId,
        "sha256": receipt.sha256,
        "relayed": receipt.relayed,
        "bytesBase64": receipt.bytes.base64EncodedString(),
      ] as [String: Any])
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("streamSubscribeExact") {
      (
        peerHandle: String,
        target: [String: String],
        payloadKind: String,
        requestJson: String
      ) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let peer = try self.requirePeer(peerHandle)
      let exact = try Self.exactTarget(target, domainId: peer.domainId())
      let kind = try Self.streamPayloadKind(payloadKind)
      // Web Wasm uses `from`; UniFFI AukiStreamRequest uses `readFrom`.
      let request = try streamRequestFromJson(
        json: Self.normalizeStreamRequestJson(requestJson)
      )
      let subscription = try await AukiStreamClient(peer: peer).subscribe(
        target: exact,
        payloadKind: kind,
        request: request
      )
      let id = self.newId("stream")
      self.streams[id] = subscription
      return id
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("streamNext") { (subscriptionId: String) -> String? in
      #if canImport(auki_sdk_swiftFFI)
      guard let subscription = self.streams[subscriptionId] else {
        throw unsupported("unknown stream subscription: \(subscriptionId)")
      }
      guard let next = try await subscription.next() else {
        return nil
      }
      return try Self.encodeStreamNext(next)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("streamCancel") { (subscriptionId: String) in
      #if canImport(auki_sdk_swiftFFI)
      guard let subscription = self.streams.removeValue(forKey: subscriptionId) else {
        return
      }
      try await subscription.cancel()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("messageOpenExact") {
      (peerHandle: String, target: [String: String], channelJson: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let peer = try self.requirePeer(peerHandle)
      let exact = try Self.exactTarget(target, domainId: peer.domainId())
      let channel = try Self.messageChannel(channelJson)
      let sender = try await AukiMessageClient(peer: peer).openExact(
        target: exact,
        channel: channel
      )
      let id = self.newId("message")
      self.messageSenders[id] = sender
      return id
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("messageSend") {
      (senderHandle: String, type: String, timestampNs: String, payloadBase64: String) in
      #if canImport(auki_sdk_swiftFFI)
      guard let sender = self.messageSenders[senderHandle] else {
        throw unsupported("unknown message sender: \(senderHandle)")
      }
      guard let timestamp = Int64(timestampNs) else {
        throw unsupported("timestampNs must be an integer string")
      }
      let payload: Data
      if payloadBase64.isEmpty {
        payload = Data()
      } else if let decoded = Data(base64Encoded: payloadBase64) {
        payload = decoded
      } else {
        throw unsupported("payloadBase64 is not valid base64")
      }
      try await sender.send(
        messageType: type,
        timestampNs: timestamp,
        payload: payload
      )
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("messageClose") { (senderHandle: String) in
      #if canImport(auki_sdk_swiftFFI)
      guard let sender = self.messageSenders.removeValue(forKey: senderHandle) else {
        return
      }
      try await sender.close()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("urdfModelFromXml") { (xml: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let model = try AukiUrdfModel.fromXml(xml: xml)
      let id = self.newId("urdf")
      self.urdfModels[id] = model
      return id
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("urdfJointCount") { (handle: String) -> Int in
      #if canImport(auki_sdk_swiftFFI)
      return Int(try self.requireUrdf(handle).jointCount())
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("urdfResolve") { (handle: String, angles: [Double]) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let floats = angles.map { Float($0) }
      let links = try self.requireUrdf(handle).resolve(angles: floats)
      return try Self.encodeUrdfLinks(links)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("urdfResolveIdentity") { (handle: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let links = try self.requireUrdf(handle).resolveIdentityPose()
      return try Self.encodeUrdfLinks(links)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("urdfModelFree") { (handle: String) in
      #if canImport(auki_sdk_swiftFFI)
      self.urdfModels.removeValue(forKey: handle)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("shutdown") { (peerHandle: String) in
      #if canImport(auki_sdk_swiftFFI)
      let leftover = self.messageSenders
      self.messageSenders.removeAll()
      for sender in leftover.values {
        try? await sender.close()
      }
      if let peer = self.peers.removeValue(forKey: peerHandle) {
        try await peer.shutdown()
      }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("waitStopped") { (peerHandle: String) in
      #if canImport(auki_sdk_swiftFFI)
      try await withAuthErrors { try await self.requirePeer(peerHandle).waitStopped() }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }
  }

  #if canImport(auki_sdk_swiftFFI)
  private func startPeer(
    sessionId: String,
    domainId: String,
    mode: AukiDiscoveryMode?
  ) async throws -> String {
    let session = try sessions.session(sessionId)
    let identity = AukiPeerIdentity.generate()
    let peer: AukiPeer
    if let mode {
      peer = try await session.startPeerWithDiscovery(
        domainId: domainId,
        identity: identity,
        mode: mode
      )
    } else {
      peer = try await session.startPeer(domainId: domainId, identity: identity)
    }
    let id = newId("peer")
    peers[id] = peer
    identities[id] = identity
    return id
  }

  private func requirePeer(_ peerHandle: String) throws -> AukiPeer {
    guard let peer = peers[peerHandle] else {
      throw unsupported("unknown peer: \(peerHandle)")
    }
    return peer
  }

  private func requireUrdf(_ handle: String) throws -> AukiUrdfModel {
    guard let model = urdfModels[handle] else {
      throw unsupported("unknown urdf model: \(handle)")
    }
    return model
  }

  private static func domainListQuery(_ json: String) throws -> AukiDomainListQuery {
    let value: ExpoDomainListQueryPayload = try decodeDataJson(json)
    return AukiDomainListQuery(
      organization: value.organization,
      domainServerId: value.domainServerId,
      limit: value.limit,
      offset: value.offset
    )
  }

  private static func dataListQuery(_ json: String) throws -> AukiDataListQuery {
    let value: ExpoDataListQueryPayload = try decodeDataJson(json)
    return AukiDataListQuery(ids: value.ids ?? [], name: value.name, dataType: value.dataType)
  }

  private static func dataWriteTarget(_ json: String) throws -> AukiDataWriteTarget {
    let value: ExpoDataWriteTargetPayload = try decodeDataJson(json)
    switch (value.id, value.name, value.dataType) {
    case let (.some(id), .none, .none): return .byId(id: id)
    case let (.none, .some(name), .some(dataType)):
      return .named(name: name, dataType: dataType)
    default:
      throw ExpoDataFailure(
        kind: "input",
        status: nil,
        message: "provide id or both name and dataType"
      )
    }
  }

  private static func transferOptions(_ json: String) throws -> AukiTransferOptions {
    let value: ExpoTransferOptionsPayload = try decodeDataJson(json)
    return AukiTransferOptions(maxBytes: value.maxBytes, maxChunkBytes: value.maxChunkBytes)
  }

  private static func decodeDataJson<T: Decodable>(_ json: String) throws -> T {
    do { return try JSONDecoder().decode(T.self, from: Data(json.utf8)) }
    catch {
      throw ExpoDataFailure(kind: "input", status: nil, message: "invalid Domain data options")
    }
  }

  private static func mapDomainPage(_ page: AukiDomainPage) -> [String: Any] {
    [
      "domains": page.domains.map(mapDomainSummary),
      "total": Double(page.total),
      "limit": page.limit,
      "offset": page.offset,
    ]
  }

  private static func mapDomainSummary(_ domain: AukiDomainSummary) -> [String: Any] {
    [
      "id": domain.id,
      "name": domain.name,
      "organization_id": nullable(domain.organizationId),
    ]
  }

  private static func mapPortalDomain(_ value: AukiPortalDomain) -> [String: Any] {
    [
      "id": value.id,
      "name": value.name,
      "organization_id": nullable(value.organizationId),
      "is_default": value.isDefault,
      "added_to_domain_at": value.addedToDomainAt,
    ]
  }

  private static func mapPortal(_ value: AukiPortal) -> [String: Any] {
    [
      "id": value.id,
      "short_id": value.shortId,
      "name": value.name,
      "size": value.size,
      "organization_id": nullable(value.organizationId),
      "default_domain_id": nullable(value.defaultDomainId),
      "redirect_url": nullable(value.redirectUrl),
      "created_at": value.createdAt,
      "updated_at": value.updatedAt,
    ]
  }

  private static func mapPortalPose(_ value: AukiPortalPose) -> [String: Any] {
    [
      "id": value.id,
      "short_id": value.shortId,
      "domain_id": value.domainId,
      "reported_size": value.reportedSize,
      "px": value.px,
      "py": value.py,
      "pz": value.pz,
      "rx": value.rx,
      "ry": value.ry,
      "rz": value.rz,
      "rw": value.rw,
      "latitude": nullable(value.latitude),
      "longitude": nullable(value.longitude),
      "altitude": nullable(value.altitude),
      "vertical_accuracy": nullable(value.verticalAccuracy),
      "horizontal_accuracy": nullable(value.horizontalAccuracy),
      "gps_timestamp": nullable(value.gpsTimestamp),
      "scanner_device_id": value.scannerDeviceId,
      "scanner_device_name": value.scannerDeviceName,
      "scanner_device_model": value.scannerDeviceModel,
      "placed_at": value.placedAt,
    ]
  }

  private static func mapDataMetadata(_ value: AukiDataMetadata) -> [String: Any] {
    [
      "id": value.id,
      "domain_id": value.domainId,
      "name": value.name,
      "data_type": value.dataType,
      "size": Double(value.size),
      "created_at": value.createdAt,
      "updated_at": value.updatedAt,
    ]
  }

  private static func nullable<T>(_ value: T?) -> Any {
    if let value { return value }
    return NSNull()
  }

  private static func encodeUrdfLinks(_ links: [AukiUrdfLinkTransform]) throws -> String {
    let mapped: [[String: Any]] = links.map { link in
      var entry: [String: Any] = [
        "linkName": link.linkName,
        "transform": link.transform.map { Double($0) },
      ]
      if let meshPath = link.meshPath {
        entry["meshPath"] = meshPath
      } else {
        entry["meshPath"] = NSNull()
      }
      if let rgba = link.colorRgba {
        entry["colorRgba"] = rgba.map { Double($0) }
      } else {
        entry["colorRgba"] = NSNull()
      }
      return entry
    }
    return try jsonString(mapped)
  }

  private static func mapCandidate(_ candidate: AukiDiscoveryCandidate) -> [String: Any] {
    var mapped: [String: Any] = [
      "peerId": candidate.peerId,
      "routes": candidate.routes,
      "servedProtocols": candidate.servedProtocols,
      "expiresAt": candidate.expiresAt,
      "source": String(describing: candidate.source),
    ]
    if let subjectId = candidate.subjectId {
      mapped["subjectId"] = subjectId
    }
    if let peerType = candidate.peerType {
      mapped["peerType"] = peerType
    }
    return mapped
  }

  private static func exactTarget(
    _ target: [String: String],
    domainId: String
  ) throws -> AukiPeerTarget {
    guard let peerId = target["peerId"], let route = target["route"] else {
      throw unsupported("exact target requires peerId and route")
    }
    return AukiPeerTarget(
      domainId: target["domainId"] ?? domainId,
      peerId: peerId,
      route: route
    )
  }

  private static func messageChannel(_ json: String) throws -> AukiMessageChannel {
    guard let data = json.data(using: .utf8) else {
      throw unsupported("message channel JSON is not UTF-8")
    }
    guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
      throw unsupported("message channel JSON must be an object")
    }
    guard
      let ownerPeerId = object["owner_peer_id"] as? String,
      let resourceId = object["resource_id"] as? String,
      let clockObject = object["clock"] as? [String: Any],
      let clockPeerId = clockObject["peer_id"] as? String,
      let clockId = clockObject["id"] as? String,
      let clockHash = clockObject["hash"] as? String
    else {
      throw unsupported(
        "message channel JSON requires owner_peer_id, resource_id, and clock"
      )
    }
    return AukiMessageChannel(
      ownerPeerId: ownerPeerId,
      resourceId: resourceId,
      clock: AukiMessageClockReference(
        peerId: clockPeerId,
        id: clockId,
        hash: clockHash
      )
    )
  }

  private static func registryKind(_ raw: String) throws -> AukiRegistryKind {
    switch raw {
    case "device_model":
      return .deviceModel
    case "sensor":
      return .sensor
    case "clock":
      return .clock
    case "frame":
      return .frame
    case "detector":
      return .detector
    case "map":
      return .map
    default:
      throw unsupported("unsupported registry kind: \(raw)")
    }
  }

  private static func streamPayloadKind(_ raw: String) throws -> AukiStreamPayloadKind {
    switch raw {
    case "pose":
      return .pose
    case "joint_encoders":
      return .jointEncoders
    case "camera":
      return .camera
    case "point_cloud":
      return .pointCloud
    case "audio":
      return .audio
    case "scalar":
      return .scalar
    case "detection":
      return .detection
    case "map":
      return .map
    default:
      throw unsupported("unsupported stream payloadKind: \(raw)")
    }
  }

  /// Map web-shaped Stream request JSON (`from`) onto UniFFI (`readFrom`).
  private static func normalizeStreamRequestJson(_ json: String) throws -> String {
    guard let data = json.data(using: .utf8) else {
      throw unsupported("stream request JSON is not UTF-8")
    }
    guard var object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
      throw unsupported("stream request JSON must be an object")
    }
    if object["readFrom"] == nil, let from = object["from"] {
      object["readFrom"] = from
      object.removeValue(forKey: "from")
    }
    if object["readFrom"] == nil {
      object["readFrom"] = ["kind": "latest"]
    }
    return try jsonString(object)
  }

  private static func encodeStreamNext(_ next: AukiStreamNext) throws -> String {
    switch next {
    case .entry(let entry):
      return try jsonString([
        "kind": "entry",
        "entry": [
          "timestampNs": String(entry.timestampNs),
          "sequence": String(entry.sequence),
          "payloadBase64": entry.payload.base64EncodedString(),
        ] as [String: Any],
      ])
    case .end(let reason):
      return try jsonString([
        "kind": "end",
        "reason": encodeEndReason(reason),
        "entry": NSNull(),
      ])
    }
  }

  private static func encodeEndReason(_ reason: AukiStreamEndReason) -> [String: Any] {
    switch reason {
    case .sourceEnded:
      return ["kind": "source_ended"]
    case .producerShuttingDown:
      return ["kind": "producer_shutting_down"]
    case .sessionEnded:
      return ["kind": "session_ended"]
    case .producerError(let detail):
      return ["kind": "producer_error", "detail": detail]
    }
  }

  private static func jsonString(_ object: Any) throws -> String {
    let data = try JSONSerialization.data(withJSONObject: object)
    guard let string = String(data: data, encoding: .utf8) else {
      throw unsupported("failed to encode JSON")
    }
    return string
  }
  #endif

  private func newId(_ prefix: String) -> String {
    "\(prefix)_\(UUID().uuidString)"
  }
}

#if canImport(auki_sdk_swiftFFI)
private struct ExpoDomainListQueryPayload: Decodable {
  let organization: String?
  let domainServerId: String?
  let limit: UInt32?
  let offset: UInt32?
}

private struct ExpoDataListQueryPayload: Decodable {
  let ids: [String]?
  let name: String?
  let dataType: String?
}

private struct ExpoDataWriteTargetPayload: Decodable {
  let id: String?
  let name: String?
  let dataType: String?
}

private struct ExpoTransferOptionsPayload: Decodable {
  let maxBytes: UInt64?
  let maxChunkBytes: UInt64?
}
#endif

private func unsupported(_ message: String) -> Exception {
  Exception(name: "AukiSdkExpoUnsupported", description: message)
}
