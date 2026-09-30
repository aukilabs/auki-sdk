package expo.modules.aukisdkexpo

import uniffi.auki_sdk_uniffi.AukiCancellation
import uniffi.auki_sdk_uniffi.AukiDomainFleet

internal class ExpoFleetRegistry {
  private val lock = Any()
  private val clients = HashMap<String, AukiDomainFleet>()
  private val operations = HashMap<String, AukiCancellation>()
  private val cancelledBeforeStart = LinkedHashSet<String>()

  fun insert(client: AukiDomainFleet, id: String) {
    synchronized(lock) { clients[id] = client }
  }

  fun existing(id: String): AukiDomainFleet? = synchronized(lock) { clients[id] }

  fun client(id: String): AukiDomainFleet =
    existing(id) ?: throw ExpoFleetFailure("closed", null, "closed", "Fleet client is closed")

  fun remove(id: String) {
    synchronized(lock) { clients.remove(id) }
  }

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
