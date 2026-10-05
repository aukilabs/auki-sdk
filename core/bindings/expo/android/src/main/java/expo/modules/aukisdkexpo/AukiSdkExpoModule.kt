package expo.modules.aukisdkexpo

import expo.modules.kotlin.functions.Coroutine
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import java.util.UUID
import uniffi.auki_sdk_uniffi.AukiBlobClient
import uniffi.auki_sdk_uniffi.AukiCatalogClient
import uniffi.auki_sdk_uniffi.AukiCatalogResourceVariant
import uniffi.auki_sdk_uniffi.AukiDataDownloadInterface
import uniffi.auki_sdk_uniffi.AukiDataUploadInterface
import uniffi.auki_sdk_uniffi.AukiDiscoveryMode
import uniffi.auki_sdk_uniffi.AukiDomainDataInterface
import uniffi.auki_sdk_uniffi.AukiDomainFleetInterface
import uniffi.auki_sdk_uniffi.AukiDomainJobsInterface
import uniffi.auki_sdk_uniffi.AukiInfoClient
import uniffi.auki_sdk_uniffi.AukiMessageClient
import uniffi.auki_sdk_uniffi.AukiMessageSender
import uniffi.auki_sdk_uniffi.AukiMessageSenderInterface
import uniffi.auki_sdk_uniffi.AukiPeer
import uniffi.auki_sdk_uniffi.AukiPeerIdentity
import uniffi.auki_sdk_uniffi.AukiRegistryClient
import uniffi.auki_sdk_uniffi.AukiSession
import uniffi.auki_sdk_uniffi.AukiStreamClient
import uniffi.auki_sdk_uniffi.AukiStreamSubscription
import uniffi.auki_sdk_uniffi.AukiUrdfModel
import uniffi.auki_sdk_uniffi.streamRequestFromJson
import uniffi.auki_sdk_uniffi.uniffiEnsureInitialized

class AukiSdkExpoModule : Module() {
  private val sessions = ExpoSessionRegistry()
  private val domainData = ExpoDomainDataRegistry()
  private val fleet = ExpoFleetRegistry()
  private val jobs = ExpoJobsRegistry()
  private val peers = HashMap<String, AukiPeer>()
  private val streams = HashMap<String, AukiStreamSubscription>()
  private val messageSenders = HashMap<String, Pair<String, AukiMessageSender>>()
  private val urdfModels = HashMap<String, AukiUrdfModel>()
  private val handles = Any()

  override fun definition() = ModuleDefinition {
    Name("AukiSdkExpo")
    Events("onZitadelSaveRequested")

    OnCreate {
      System.setProperty("jna.nosys", "false")
      System.loadLibrary("auki_sdk_uniffi")
      uniffiEnsureInitialized()
    }

    AsyncFunction("_importZitadel") Coroutine { credentialsJson: String, environmentJson: String? ->
      val id = newId("session")
      val store = ExpoZitadelStore { requestId ->
        sendEvent(
          "onZitadelSaveRequested",
          mapOf("sessionId" to id, "requestId" to requestId),
        )
      }
      val session = importZitadel(credentialsJson, environmentJson, store)
      sessions.insert(id, session, store)
      id
    }

    AsyncFunction("_zitadelCredentials") Coroutine { sessionId: String, requestId: String ->
      sessions.store(sessionId).credentialsJson(requestId)
    }

    AsyncFunction("_ackZitadelSave") Coroutine { sessionId: String, requestId: String, success: Boolean ->
      sessions.store(sessionId).acknowledge(requestId, success)
    }

    AsyncFunction("_closeSession") Coroutine { sessionId: String ->
      sessions.close(sessionId)
    }

    AsyncFunction("loginDev") Coroutine { email: String, password: String, clientId: String? ->
      val session = withAuthErrors { AukiSession.loginDev(email, password, clientId) }
      val id = newId("session")
      sessions.insert(id, session)
      id
    }

    AsyncFunction("loginWithEnvironment") Coroutine {
        apiBaseUrl: String,
        ddsBaseUrl: String,
        dmsBaseUrl: String,
        email: String,
        password: String,
        clientId: String?,
      ->
      val session = withAuthErrors {
        AukiSession.loginWithEnvironment(
          apiBaseUrl,
          ddsBaseUrl,
          dmsBaseUrl,
          email,
          password,
          clientId,
        )
      }
      val id = newId("session")
      sessions.insert(id, session)
      id
    }

    AsyncFunction("accessibleDomains") Coroutine { sessionId: String ->
      val domains = withAuthErrors { sessions.session(sessionId).accessibleDomains() }
      domains.map(::mapDomain)
    }

    AsyncFunction("domainsForPortalPage") Coroutine {
        sessionId: String,
        portal: String,
        organization: String,
        limit: Int,
        cursor: String?,
        operationId: String,
      ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        val page = withDataErrors {
          sessions.session(sessionId).domains().forPortalPage(
            portal = portal,
            limit = limit.toUInt(),
            cursor = cursor,
            organization = organization,
            cancellation = cancellation,
          )
        }
        mapOf(
          "items" to page.items.map(::mapPortalDomain),
          "next_cursor" to page.nextCursor,
          "paginated" to page.paginated,
        )
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainsPortalsPage") Coroutine {
        sessionId: String,
        domainId: String,
        limit: Int,
        cursor: String?,
        operationId: String,
      ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        val page = withDataErrors {
          sessions.session(sessionId).domains().portalsPage(
            domainId = domainId,
            limit = limit.toUInt(),
            cursor = cursor,
            cancellation = cancellation,
          )
        }
        mapOf(
          "items" to page.items.map(::mapPortal),
          "next_cursor" to page.nextCursor,
          "paginated" to page.paginated,
        )
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainsList") Coroutine { sessionId: String, queryJson: String, operationId: String ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        val query = domainListQuery(queryJson)
        val page = withDataErrors {
          sessions.session(sessionId).domains().list(query = query, cancellation = cancellation)
        }
        mapDomainPage(page)
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainsForPortal") Coroutine {
        sessionId: String,
        portal: String,
        organization: String?,
        operationId: String,
      ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        val values = withDataErrors {
          sessions.session(sessionId).domains().forPortal(
            portal = portal,
            organization = organization,
            cancellation = cancellation,
          )
        }
        values.map(::mapPortalDomain)
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainsPortals") Coroutine { sessionId: String, domainId: String, operationId: String ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        val values = withDataErrors {
          sessions.session(sessionId).domains().portals(
            domainId = domainId,
            cancellation = cancellation,
          )
        }
        values.map(::mapPortal)
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainsPortal") Coroutine {
        sessionId: String,
        domainId: String,
        portal: String,
        operationId: String,
      ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        val value = withDataErrors {
          sessions.session(sessionId).domains().portal(
            domainId = domainId,
            portal = portal,
            cancellation = cancellation,
          )
        }
        mapPortal(value)
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainDataOpen") Coroutine { sessionId: String, domainId: String ->
      val client = withDataErrors { sessions.session(sessionId).data(domainId) }
      val id = newId("data")
      domainData.insertClient(client, id)
      id
    }

    AsyncFunction("domainDataList") Coroutine { clientId: String, queryJson: String, operationId: String ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        val query = dataListQuery(queryJson)
        val values = withDataErrors {
          domainData.client(clientId).list(query = query, cancellation = cancellation)
        }
        values.map(::mapDataMetadata)
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainDataGet") Coroutine { clientId: String, dataId: String, operationId: String ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        val value = withDataErrors {
          domainData.client(clientId).get(dataId = dataId, cancellation = cancellation)
        }
        mapDataMetadata(value)
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainDataRead") Coroutine { clientId: String, dataId: String, operationId: String ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        val bytes = withDataErrors {
          domainData.client(clientId).read(dataId = dataId, cancellation = cancellation)
        }
        encodeBase64(bytes)
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainDataWrite") Coroutine {
        clientId: String,
        targetJson: String,
        bytesBase64: String,
        operationId: String,
      ->
      val bytes = decodeBase64(bytesBase64)
        ?: throw ExpoDataFailure("input", null, "", "Data bytes are not base64")
      val cancellation = domainData.beginOperation(operationId)
      try {
        val target = dataWriteTarget(targetJson)
        val value = withDataErrors {
          domainData.client(clientId).write(
            target = target,
            bytes = bytes,
            cancellation = cancellation,
          )
        }
        mapDataMetadata(value)
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainDataDelete") Coroutine { clientId: String, dataId: String, operationId: String ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        withDataErrors {
          domainData.client(clientId).delete(dataId = dataId, cancellation = cancellation)
        }
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainDataPoses") Coroutine { clientId: String, operationId: String ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        val values = withDataErrors {
          domainData.client(clientId).poses(cancellation = cancellation)
        }
        values.map(::mapPortalPose)
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainDataPose") Coroutine { clientId: String, portal: String, operationId: String ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        val value = withDataErrors {
          domainData.client(clientId).pose(portal = portal, cancellation = cancellation)
        }
        mapPortalPose(value)
      } finally {
        domainData.finishOperation(operationId)
      }
    }

    AsyncFunction("domainDataClose") Coroutine { clientId: String ->
      val client = domainData.removeClient(clientId) ?: return@Coroutine Unit
      withDataErrors { (client as AukiDomainDataInterface).close() }
    }

    AsyncFunction("dataOperationCancel") Coroutine { operationId: String ->
      domainData.cancelOperation(operationId)
    }

    AsyncFunction("dataDownloadStart") Coroutine {
        clientId: String,
        dataId: String,
        optionsJson: String,
        operationId: String,
      ->
      val cancellation = domainData.beginOperation(operationId)
      try {
        val transfer = withDataErrors {
          domainData.client(clientId).startDownload(
            dataId = dataId,
            options = transferOptions(optionsJson),
            cancellation = cancellation,
          )
        }
        val id = newId("download")
        domainData.insertDownload(transfer, id, operationId)
        id
      } catch (error: Exception) {
        domainData.finishOperation(operationId)
        throw error
      }
    }

    AsyncFunction("dataDownloadNext") Coroutine { downloadId: String ->
      val bytes = withDataErrors { domainData.download(downloadId).next() }
      bytes?.let(::encodeBase64)
    }

    AsyncFunction("dataDownloadCancel") Coroutine { downloadId: String ->
      withDataErrors { domainData.download(downloadId).cancel() }
    }

    AsyncFunction("dataDownloadClose") Coroutine { downloadId: String ->
      val transfer = domainData.removeDownload(downloadId) ?: return@Coroutine Unit
      withDataErrors { (transfer as AukiDataDownloadInterface).close() }
    }

    AsyncFunction("dataUploadStart") Coroutine {
        clientId: String,
        targetJson: String,
        size: Double,
        optionsJson: String,
        operationId: String,
      ->
      if (!size.isFinite() || size.toLong().toDouble() != size || size < 1.0 || size > 9_007_199_254_740_991.0) {
        throw ExpoDataFailure("input", null, "", "size must be a positive safe integer")
      }
      val cancellation = domainData.beginOperation(operationId)
      try {
        val transfer = withDataErrors {
          domainData.client(clientId).startUpload(
            target = dataWriteTarget(targetJson),
            size = size.toLong().toULong(),
            options = transferOptions(optionsJson),
            cancellation = cancellation,
          )
        }
        val id = newId("upload")
        domainData.insertUpload(transfer, id, operationId)
        id
      } catch (error: Exception) {
        domainData.finishOperation(operationId)
        throw error
      }
    }

    AsyncFunction("dataUploadNextMaximum") Coroutine { uploadId: String ->
      withDataErrors { domainData.upload(uploadId).nextMaximum() }?.toDouble()
    }

    AsyncFunction("dataUploadPush") Coroutine { uploadId: String, bytesBase64: String ->
      val bytes = decodeBase64(bytesBase64)
        ?: throw ExpoDataFailure("input", null, "", "Upload chunk is not base64")
      withDataErrors { domainData.upload(uploadId).push(bytes) }
    }

    AsyncFunction("dataUploadResult") Coroutine { uploadId: String ->
      mapDataMetadata(withDataErrors { domainData.upload(uploadId).result() })
    }

    AsyncFunction("dataUploadCancel") Coroutine { uploadId: String ->
      withDataErrors { domainData.upload(uploadId).cancel() }
    }

    AsyncFunction("dataUploadClose") Coroutine { uploadId: String ->
      val transfer = domainData.removeUpload(uploadId) ?: return@Coroutine Unit
      withDataErrors { (transfer as AukiDataUploadInterface).close() }
    }

    AsyncFunction("fleetOpen") Coroutine { sessionId: String, domainId: String ->
      val client = withFleetErrors { sessions.session(sessionId).fleet(domainId) }
      val id = newId("fleet")
      fleet.insert(client, id)
      id
    }

    AsyncFunction("fleetList") Coroutine { clientId: String, queryJson: String, operationId: String ->
      val cancellation = fleet.beginOperation(operationId)
      try {
        withFleetErrors {
          fleet.client(clientId).listJson(queryJson = queryJson, cancellation = cancellation)
        }
      } finally {
        fleet.finishOperation(operationId)
      }
    }

    AsyncFunction("fleetComputePool") Coroutine { clientId: String, queryJson: String, operationId: String ->
      val cancellation = fleet.beginOperation(operationId)
      try {
        withFleetErrors {
          fleet.client(clientId).computePoolJson(queryJson = queryJson, cancellation = cancellation)
        }
      } finally {
        fleet.finishOperation(operationId)
      }
    }

    AsyncFunction("fleetOperationCancel") Coroutine { operationId: String ->
      fleet.cancelOperation(operationId)
    }

    AsyncFunction("fleetClose") Coroutine { clientId: String ->
      val client = fleet.existing(clientId) ?: return@Coroutine Unit
      withFleetErrors { (client as AukiDomainFleetInterface).close() }
      fleet.remove(clientId)
    }

    AsyncFunction("jobsOpen") Coroutine { sessionId: String, domainId: String ->
      val client = withJobsErrors { sessions.session(sessionId).jobs(domainId) }
      val id = newId("jobs")
      jobs.insert(client, id)
      id
    }

    AsyncFunction("jobsEstimate") Coroutine { clientId: String, specJson: String, operationId: String ->
      val cancellation = jobs.beginOperation(operationId)
      try {
        withJobsErrors {
          jobs.client(clientId).estimateJson(specJson = specJson, cancellation = cancellation)
        }
      } finally {
        jobs.finishOperation(operationId)
      }
    }

    AsyncFunction("jobsSubmit") Coroutine { clientId: String, specJson: String, operationId: String ->
      val cancellation = jobs.beginOperation(operationId)
      try {
        withJobsErrors {
          jobs.client(clientId).submitJson(specJson = specJson, cancellation = cancellation)
        }
      } finally {
        jobs.finishOperation(operationId)
      }
    }

    AsyncFunction("jobsSubmitWithKey") Coroutine {
        clientId: String,
        specJson: String,
        idempotencyKey: String,
        operationId: String,
      ->
      val cancellation = jobs.beginOperation(operationId)
      try {
        withJobsErrors {
          jobs.client(clientId).submitWithKeyJson(
            specJson = specJson,
            idempotencyKey = idempotencyKey,
            cancellation = cancellation,
          )
        }
      } finally {
        jobs.finishOperation(operationId)
      }
    }

    AsyncFunction("jobsList") Coroutine { clientId: String, queryJson: String, operationId: String ->
      val cancellation = jobs.beginOperation(operationId)
      try {
        withJobsErrors {
          jobs.client(clientId).listJson(queryJson = queryJson, cancellation = cancellation)
        }
      } finally {
        jobs.finishOperation(operationId)
      }
    }

    AsyncFunction("jobsGet") Coroutine { clientId: String, jobId: String, operationId: String ->
      val cancellation = jobs.beginOperation(operationId)
      try {
        withJobsErrors {
          jobs.client(clientId).getJson(jobId = jobId, cancellation = cancellation)
        }
      } finally {
        jobs.finishOperation(operationId)
      }
    }

    AsyncFunction("jobsCancel") Coroutine { clientId: String, jobId: String, operationId: String ->
      val cancellation = jobs.beginOperation(operationId)
      try {
        withJobsErrors {
          jobs.client(clientId).cancelJson(jobId = jobId, cancellation = cancellation)
        }
      } finally {
        jobs.finishOperation(operationId)
      }
    }

    AsyncFunction("jobsOperationCancel") Coroutine { operationId: String ->
      jobs.cancelOperation(operationId)
    }

    AsyncFunction("jobsClose") Coroutine { clientId: String ->
      val client = jobs.remove(clientId) ?: return@Coroutine Unit
      withJobsErrors { (client as AukiDomainJobsInterface).close() }
    }

    AsyncFunction("startPeer") Coroutine { sessionId: String, domainId: String ->
      withAuthErrors { startPeer(sessionId, domainId, null) }
    }

    AsyncFunction("startPeerWithDiscovery") Coroutine { sessionId: String, domainId: String, mode: String ->
      val discovery = if (mode == "DiscoverAndAdvertise") {
        AukiDiscoveryMode.DISCOVER_AND_ADVERTISE
      } else {
        AukiDiscoveryMode.DISCOVER_ONLY
      }
      withAuthErrors { startPeer(sessionId, domainId, discovery) }
    }

    AsyncFunction("peerId") Coroutine { peerHandle: String ->
      requirePeer(peerHandle).peerId()
    }

    AsyncFunction("domainId") Coroutine { peerHandle: String ->
      requirePeer(peerHandle).domainId()
    }

    AsyncFunction("discover") Coroutine { peerHandle: String ->
      requirePeer(peerHandle).discover().map(::mapCandidate)
    }

    AsyncFunction("discoverProtocol") Coroutine { peerHandle: String, protocolId: String ->
      requirePeer(peerHandle).discoverProtocol(protocolId).map(::mapCandidate)
    }

    AsyncFunction("infoFetchExact") Coroutine { peerHandle: String, target: Map<String, String> ->
      val peer = requirePeer(peerHandle)
      AukiInfoClient(peer).fetchExact(exactTarget(target, peer.domainId()))
    }

    AsyncFunction("catalogFetchResourcesExact") Coroutine {
        peerHandle: String,
        target: Map<String, String>,
        variants: List<String>,
      ->
      val peer = requirePeer(peerHandle)
      // iOS discards the JS variant list and requests every Catalog family.
      @Suppress("UNUSED_VARIABLE")
      val ignored = variants
      AukiCatalogClient(peer).fetchResourcesExact(
        exactTarget(target, peer.domainId()),
        emptyList<AukiCatalogResourceVariant>(),
      )
    }

    AsyncFunction("registryListExact") Coroutine { peerHandle: String, target: Map<String, String>, kind: String ->
      val peer = requirePeer(peerHandle)
      val entries = AukiRegistryClient(peer).listExact(
        exactTarget(target, peer.domainId()),
        registryKind(kind),
      )
      jsonString(entries.map { mapOf("id" to it.id, "hash" to it.hash) })
    }

    AsyncFunction("registryFetchExact") Coroutine {
        peerHandle: String,
        target: Map<String, String>,
        kind: String,
        id: String,
        hash: String,
      ->
      val peer = requirePeer(peerHandle)
      AukiRegistryClient(peer).fetchExact(
        target = exactTarget(target, peer.domainId()),
        kind = registryKind(kind),
        id = id,
        hash = hash,
      )
    }

    AsyncFunction("blobFetchExact") Coroutine { peerHandle: String, target: Map<String, String>, sha256: String ->
      val peer = requirePeer(peerHandle)
      val receipt = AukiBlobClient(peer).fetchExact(exactTarget(target, peer.domainId()), sha256)
      jsonString(
        mapOf(
          "peerId" to receipt.remotePeerId,
          "sha256" to receipt.sha256,
          "relayed" to receipt.relayed,
          "bytesBase64" to encodeBase64(receipt.bytes),
        ),
      )
    }

    AsyncFunction("streamSubscribeExact") Coroutine {
        peerHandle: String,
        target: Map<String, String>,
        payloadKind: String,
        requestJson: String,
      ->
      val peer = requirePeer(peerHandle)
      val request = streamRequestFromJson(normalizeStreamRequestJson(requestJson))
      val subscription = AukiStreamClient(peer).subscribe(
        target = exactTarget(target, peer.domainId()),
        payloadKind = streamPayloadKind(payloadKind),
        request = request,
      )
      val id = newId("stream")
      synchronized(handles) { streams[id] = subscription }
      id
    }

    AsyncFunction("streamNext") Coroutine { subscriptionId: String ->
      val subscription = synchronized(handles) { streams[subscriptionId] }
        ?: throw ExpoUnsupported("unknown stream subscription: $subscriptionId")
      val next = subscription.next() ?: return@Coroutine null
      encodeStreamNext(next)
    }

    AsyncFunction("streamCancel") Coroutine { subscriptionId: String ->
      val subscription = synchronized(handles) { streams.remove(subscriptionId) } ?: return@Coroutine Unit
      subscription.cancel()
    }

    AsyncFunction("messageOpenExact") Coroutine {
        peerHandle: String,
        target: Map<String, String>,
        channelJson: String,
      ->
      val peer = requirePeer(peerHandle)
      val sender = AukiMessageClient(peer).openExact(
        target = exactTarget(target, peer.domainId()),
        channel = messageChannel(channelJson),
      )
      val id = newId("message")
      synchronized(handles) { messageSenders[id] = peerHandle to sender }
      id
    }

    AsyncFunction("messageSend") Coroutine {
        senderHandle: String,
        type: String,
        timestampNs: String,
        payloadBase64: String,
      ->
      val sender = synchronized(handles) { messageSenders[senderHandle] }?.second
        ?: throw ExpoUnsupported("unknown message sender: $senderHandle")
      val timestamp = timestampNs.toLongOrNull()
        ?: throw ExpoUnsupported("timestampNs must be an integer string")
      val payload = if (payloadBase64.isEmpty()) {
        ByteArray(0)
      } else {
        decodeBase64(payloadBase64) ?: throw ExpoUnsupported("payloadBase64 is not valid base64")
      }
      (sender as AukiMessageSenderInterface).send(type, timestamp, payload)
    }

    AsyncFunction("messageClose") Coroutine { senderHandle: String ->
      val sender = synchronized(handles) { messageSenders.remove(senderHandle) }?.second
        ?: return@Coroutine Unit
      (sender as AukiMessageSenderInterface).close()
    }

    AsyncFunction("urdfModelFromXml") Coroutine { xml: String ->
      val model = AukiUrdfModel.fromXml(xml)
      val id = newId("urdf")
      synchronized(handles) { urdfModels[id] = model }
      id
    }

    AsyncFunction("urdfJointCount") Coroutine { handle: String ->
      requireUrdf(handle).jointCount().toInt()
    }

    AsyncFunction("urdfResolve") Coroutine { handle: String, angles: List<Double> ->
      val links = requireUrdf(handle).resolve(angles.map { it.toFloat() })
      encodeUrdfLinks(links)
    }

    AsyncFunction("urdfResolveIdentity") Coroutine { handle: String ->
      encodeUrdfLinks(requireUrdf(handle).resolveIdentityPose())
    }

    AsyncFunction("urdfModelFree") Coroutine { handle: String ->
      synchronized(handles) { urdfModels.remove(handle) }
      Unit
    }

    AsyncFunction("shutdown") Coroutine { peerHandle: String ->
      val owned = synchronized(handles) {
        messageSenders.filterValues { it.first == peerHandle }.keys.toList()
      }
      for (id in owned) {
        val sender = synchronized(handles) { messageSenders.remove(id) }?.second ?: continue
        try {
          (sender as AukiMessageSenderInterface).close()
        } catch (_: Exception) {
        }
      }
      val peer = synchronized(handles) { peers.remove(peerHandle) }
      if (peer != null) peer.shutdown()
    }

    AsyncFunction("waitStopped") Coroutine { peerHandle: String ->
      withAuthErrors { requirePeer(peerHandle).waitStopped() }
    }
  }

  private suspend fun startPeer(sessionId: String, domainId: String, mode: AukiDiscoveryMode?): String {
    val session = sessions.session(sessionId)
    val identity = AukiPeerIdentity.generate()
    val peer = if (mode == null) {
      session.startPeer(domainId, identity)
    } else {
      session.startPeerWithDiscovery(domainId, identity, mode)
    }
    val id = newId("peer")
    synchronized(handles) { peers[id] = peer }
    return id
  }

  private fun requirePeer(peerHandle: String): AukiPeer =
    synchronized(handles) { peers[peerHandle] }
      ?: throw ExpoUnsupported("unknown peer: $peerHandle")

  private fun requireUrdf(handle: String): AukiUrdfModel =
    synchronized(handles) { urdfModels[handle] }
      ?: throw ExpoUnsupported("unknown urdf model: $handle")

  private fun newId(prefix: String) = "${prefix}_${UUID.randomUUID()}"
}
