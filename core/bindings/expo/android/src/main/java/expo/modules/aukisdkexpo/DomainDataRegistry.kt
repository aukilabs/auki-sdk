package expo.modules.aukisdkexpo

import uniffi.auki_sdk_uniffi.AukiCancellation
import uniffi.auki_sdk_uniffi.AukiDataDownload
import uniffi.auki_sdk_uniffi.AukiDataUpload
import uniffi.auki_sdk_uniffi.AukiDomainData

internal class ExpoDomainDataRegistry {
  private val lock = Any()
  private val clients = HashMap<String, AukiDomainData>()
  private val operations = HashMap<String, AukiCancellation>()
  private val cancelledBeforeStart = LinkedHashSet<String>()
  private val downloads = HashMap<String, DownloadEntry>()
  private val uploads = HashMap<String, UploadEntry>()

  private class DownloadEntry(val transfer: AukiDataDownload, val operationId: String)
  private class UploadEntry(val transfer: AukiDataUpload, val operationId: String)

  fun insertClient(client: AukiDomainData, id: String) {
    synchronized(lock) { clients[id] = client }
  }

  fun client(id: String): AukiDomainData =
    synchronized(lock) { clients[id] }
      ?: throw ExpoDataFailure("closed", null, "", "Domain data client is closed")

  fun removeClient(id: String): AukiDomainData? = synchronized(lock) { clients.remove(id) }

  fun beginOperation(id: String): AukiCancellation {
    val cancellation = AukiCancellation()
    val cancelled = synchronized(lock) {
      operations[id] = cancellation
      cancelledBeforeStart.remove(id)
    }
    if (cancelled) cancellation.cancel()
    return cancellation
  }

  fun finishOperation(id: String) {
    synchronized(lock) { operations.remove(id) }
  }

  fun cancelOperation(id: String) {
    val cancellation = synchronized(lock) {
      operations[id] ?: run {
        if (cancelledBeforeStart.size < 1_024) cancelledBeforeStart.add(id)
        null
      }
    }
    cancellation?.cancel()
  }

  fun insertDownload(transfer: AukiDataDownload, id: String, operationId: String) {
    synchronized(lock) { downloads[id] = DownloadEntry(transfer, operationId) }
  }

  fun download(id: String): AukiDataDownload =
    synchronized(lock) { downloads[id] }?.transfer
      ?: throw ExpoDataFailure("closed", null, "", "Domain data download is closed")

  fun removeDownload(id: String): AukiDataDownload? {
    val entry = synchronized(lock) { downloads.remove(id) } ?: return null
    finishOperation(entry.operationId)
    return entry.transfer
  }

  fun insertUpload(transfer: AukiDataUpload, id: String, operationId: String) {
    synchronized(lock) { uploads[id] = UploadEntry(transfer, operationId) }
  }

  fun upload(id: String): AukiDataUpload =
    synchronized(lock) { uploads[id] }?.transfer
      ?: throw ExpoDataFailure("closed", null, "", "Domain data upload is closed")

  fun removeUpload(id: String): AukiDataUpload? {
    val entry = synchronized(lock) { uploads.remove(id) } ?: return null
    finishOperation(entry.operationId)
    return entry.transfer
  }
}
