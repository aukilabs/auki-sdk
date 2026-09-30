package expo.modules.aukisdkexpo

import java.util.UUID
import kotlinx.coroutines.CompletableDeferred
import org.json.JSONObject
import uniffi.auki_sdk_uniffi.AukiAuthFailureKind
import uniffi.auki_sdk_uniffi.AukiPersistenceException
import uniffi.auki_sdk_uniffi.AukiSession
import uniffi.auki_sdk_uniffi.AukiZitadelCredentials
import uniffi.auki_sdk_uniffi.AukiZitadelSessionStore

internal class ExpoZitadelStore(
  private val notify: (requestId: String) -> Unit,
) : AukiZitadelSessionStore {
  private val lock = Any()
  private var pending: Pending? = null

  private class Pending(
    val id: String,
    val credentials: AukiZitadelCredentials,
    val ack: CompletableDeferred<Boolean>,
  )

  override suspend fun save(credentials: AukiZitadelCredentials) {
    val requestId = UUID.randomUUID().toString()
    val ack = CompletableDeferred<Boolean>()
    synchronized(lock) {
      if (pending != null) throw AukiPersistenceException.Failed()
      pending = Pending(requestId, credentials, ack)
    }
    notify(requestId)
    if (!ack.await()) throw AukiPersistenceException.Failed()
  }

  fun credentialsJson(requestId: String): String {
    val credentials = synchronized(lock) {
      val current = pending ?: throw ExpoAuthFailure(AukiAuthFailureKind.PERSISTENCE)
      if (current.id != requestId) throw ExpoAuthFailure(AukiAuthFailureKind.PERSISTENCE)
      current.credentials
    }
    return try {
      JSONObject()
        .put("accessToken", credentials.exposeAccessToken())
        .put("refreshToken", credentials.exposeRefreshToken())
        .put("clientId", credentials.clientId())
        .put("issuer", credentials.issuer())
        .put("accessTokenExpiresAt", credentials.accessTokenExpiresAt() ?: JSONObject.NULL)
        .toString()
    } catch (error: ExpoAuthFailure) {
      throw error
    } catch (_: Exception) {
      throw ExpoAuthFailure(AukiAuthFailureKind.PERSISTENCE)
    }
  }

  fun acknowledge(requestId: String, success: Boolean): Boolean {
    val current = synchronized(lock) {
      val current = pending ?: return false
      if (current.id != requestId) return false
      pending = null
      current
    }
    current.ack.complete(success)
    return true
  }
}

internal class ExpoSessionRegistry {
  private val lock = Any()
  private val sessions = HashMap<String, AukiSession>()
  private val stores = HashMap<String, ExpoZitadelStore>()

  fun insert(id: String, session: AukiSession, store: ExpoZitadelStore? = null) {
    synchronized(lock) {
      sessions[id] = session
      if (store != null) stores[id] = store
    }
  }

  fun session(id: String): AukiSession =
    synchronized(lock) { sessions[id] } ?: throw ExpoAuthFailure(AukiAuthFailureKind.CLOSED)

  fun store(id: String): ExpoZitadelStore =
    synchronized(lock) { stores[id] } ?: throw ExpoAuthFailure(AukiAuthFailureKind.CLOSED)

  suspend fun close(id: String) {
    val session = synchronized(lock) { sessions[id] } ?: return
    (session as uniffi.auki_sdk_uniffi.AukiSessionInterface).close()
    synchronized(lock) {
      sessions.remove(id)
      stores.remove(id)
    }
  }
}

internal fun importZitadel(
  credentialsJson: String,
  environmentJson: String?,
  store: ExpoZitadelStore,
): AukiSession {
  try {
    if (credentialsJson.toByteArray(Charsets.UTF_8).size > 2_097_152) {
      throw ExpoAuthFailure(AukiAuthFailureKind.CONFIGURATION)
    }
    val payload = JSONObject(credentialsJson)
    val credentials = AukiZitadelCredentials(
      accessToken = payload.getString("accessToken"),
      refreshToken = payload.getString("refreshToken"),
      clientId = payload.getString("clientId"),
      issuer = payload.getString("issuer"),
      accessTokenExpiresAt = payload.optNullableString("accessTokenExpiresAt"),
    )
    if (environmentJson != null) {
      val environment = JSONObject(environmentJson)
      return AukiSession.importZitadelWithEnvironment(
        apiBaseUrl = environment.getString("apiBaseUrl"),
        ddsBaseUrl = environment.getString("ddsBaseUrl"),
        dmsBaseUrl = environment.getString("dmsBaseUrl"),
        credentials = credentials,
        store = store,
      )
    }
    return AukiSession.importZitadelDev(credentials = credentials, store = store)
  } catch (error: ExpoAuthFailure) {
    throw error
  } catch (_: Exception) {
    throw ExpoAuthFailure(AukiAuthFailureKind.CONFIGURATION)
  }
}
