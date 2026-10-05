package expo.modules.aukisdkexpo

import uniffi.auki_sdk_uniffi.AukiCancellation
import uniffi.auki_sdk_uniffi.AukiDomainJobs

internal class ExpoJobsRegistry {
  private val lock = Any()
  private val clients = HashMap<String, AukiDomainJobs>()
  private val operations = HashMap<String, AukiCancellation>()
  private val cancelledBeforeStart = LinkedHashSet<String>()

  fun insert(client: AukiDomainJobs, id: String) {
    synchronized(lock) { clients[id] = client }
  }

  fun client(id: String): AukiDomainJobs =
    synchronized(lock) { clients[id] }
      ?: throw ExpoJobsFailure(
        kind = "closed",
        status = null,
        code = null,
        maximum = null,
        source = null,
        retryAfterSeconds = null,
        message = "DMS jobs client is closed",
      )

  fun remove(id: String): AukiDomainJobs? = synchronized(lock) { clients.remove(id) }

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
}
