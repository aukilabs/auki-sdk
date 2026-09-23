package expo.modules.aukisdkexpo

import android.util.Base64
import org.json.JSONArray
import org.json.JSONObject
import uniffi.auki_sdk_uniffi.AukiDataListQuery
import uniffi.auki_sdk_uniffi.AukiDataWriteTarget
import uniffi.auki_sdk_uniffi.AukiDomainListQuery
import uniffi.auki_sdk_uniffi.AukiMessageChannel
import uniffi.auki_sdk_uniffi.AukiMessageClockReference
import uniffi.auki_sdk_uniffi.AukiPeerTarget
import uniffi.auki_sdk_uniffi.AukiRegistryKind
import uniffi.auki_sdk_uniffi.AukiStreamPayloadKind
import uniffi.auki_sdk_uniffi.AukiTransferOptions

private val base64Pattern = Regex("^[A-Za-z0-9+/]*={0,2}$")

internal fun JSONObject.optNullableString(key: String): String? {
  if (!has(key) || isNull(key)) return null
  return getString(key)
}

internal fun decodeBase64(value: String): ByteArray? {
  if (value.isEmpty()) return ByteArray(0)
  if (value.length % 4 != 0 || !base64Pattern.matches(value)) return null
  return try {
    Base64.decode(value, Base64.DEFAULT)
  } catch (_: IllegalArgumentException) {
    null
  }
}

internal fun encodeBase64(bytes: ByteArray): String =
  Base64.encodeToString(bytes, Base64.NO_WRAP)

internal fun domainListQuery(json: String): AukiDomainListQuery {
  val value = decodeObject(json)
  return AukiDomainListQuery(
    organization = value.optNullableString("organization"),
    domainServerId = value.optNullableString("domainServerId"),
    limit = value.optUInt("limit"),
    offset = value.optUInt("offset"),
  )
}

internal fun dataListQuery(json: String): AukiDataListQuery {
  val value = decodeObject(json)
  return AukiDataListQuery(
    ids = value.optStringList("ids"),
    name = value.optNullableString("name"),
    dataType = value.optNullableString("dataType"),
  )
}

internal fun dataWriteTarget(json: String): AukiDataWriteTarget {
  val value = decodeObject(json)
  val id = value.optNullableString("id")
  val name = value.optNullableString("name")
  val dataType = value.optNullableString("dataType")
  return when {
    id != null && name == null && dataType == null -> AukiDataWriteTarget.ById(id)
    id == null && name != null && dataType != null -> AukiDataWriteTarget.Named(name, dataType)
    else -> throw ExpoDataFailure("input", null, "", "provide id or both name and dataType")
  }
}

internal fun transferOptions(json: String): AukiTransferOptions {
  val value = decodeObject(json)
  return AukiTransferOptions(
    maxBytes = value.optULong("maxBytes"),
    maxChunkBytes = value.optULong("maxChunkBytes"),
  )
}

internal fun exactTarget(target: Map<String, String>, domainId: String): AukiPeerTarget {
  val peerId = target["peerId"]
  val route = target["route"]
  if (peerId == null || route == null) {
    throw ExpoUnsupported("exact target requires peerId and route")
  }
  return AukiPeerTarget(
    domainId = target["domainId"] ?: domainId,
    peerId = peerId,
    route = route,
  )
}

internal fun messageChannel(json: String): AukiMessageChannel {
  val objectJson = try {
    JSONObject(json)
  } catch (_: Exception) {
    throw ExpoUnsupported("message channel JSON must be an object")
  }
  val clock = objectJson.optJSONObject("clock")
    ?: throw ExpoUnsupported("message channel JSON requires owner_peer_id, resource_id, and clock")
  val ownerPeerId = objectJson.optNullableString("owner_peer_id")
  val resourceId = objectJson.optNullableString("resource_id")
  val clockPeerId = clock.optNullableString("peer_id")
  val clockId = clock.optNullableString("id")
  val clockHash = clock.optNullableString("hash")
  if (ownerPeerId == null || resourceId == null || clockPeerId == null || clockId == null || clockHash == null) {
    throw ExpoUnsupported("message channel JSON requires owner_peer_id, resource_id, and clock")
  }
  return AukiMessageChannel(
    ownerPeerId = ownerPeerId,
    resourceId = resourceId,
    clock = AukiMessageClockReference(peerId = clockPeerId, id = clockId, hash = clockHash),
  )
}

internal fun registryKind(raw: String): AukiRegistryKind = when (raw) {
  "device_model" -> AukiRegistryKind.DEVICE_MODEL
  "sensor" -> AukiRegistryKind.SENSOR
  "clock" -> AukiRegistryKind.CLOCK
  "frame" -> AukiRegistryKind.FRAME
  "detector" -> AukiRegistryKind.DETECTOR
  "map" -> AukiRegistryKind.MAP
  else -> throw ExpoUnsupported("unsupported registry kind: $raw")
}

internal fun streamPayloadKind(raw: String): AukiStreamPayloadKind = when (raw) {
  "pose" -> AukiStreamPayloadKind.POSE
  "joint_encoders" -> AukiStreamPayloadKind.JOINT_ENCODERS
  "camera" -> AukiStreamPayloadKind.CAMERA
  "point_cloud" -> AukiStreamPayloadKind.POINT_CLOUD
  "audio" -> AukiStreamPayloadKind.AUDIO
  "scalar" -> AukiStreamPayloadKind.SCALAR
  "detection" -> AukiStreamPayloadKind.DETECTION
  "map" -> AukiStreamPayloadKind.MAP
  else -> throw ExpoUnsupported("unsupported stream payloadKind: $raw")
}

internal fun normalizeStreamRequestJson(json: String): String {
  val objectJson = try {
    JSONObject(json)
  } catch (_: Exception) {
    throw ExpoUnsupported("stream request JSON must be an object")
  }
  if (!objectJson.has("readFrom") && objectJson.has("from")) {
    objectJson.put("readFrom", objectJson.get("from"))
    objectJson.remove("from")
  }
  if (!objectJson.has("readFrom") || objectJson.isNull("readFrom")) {
    objectJson.put("readFrom", JSONObject().put("kind", "latest"))
  }
  return objectJson.toString()
}

internal fun jsonString(value: Any?): String = toJson(value).let { encoded ->
  when (encoded) {
    is JSONObject -> encoded.toString()
    is JSONArray -> encoded.toString()
    else -> throw ExpoUnsupported("failed to encode JSON")
  }
}

private fun toJson(value: Any?): Any = when (value) {
  null -> JSONObject.NULL
  is JSONObject, is JSONArray -> value
  is Map<*, *> -> JSONObject().apply {
    value.forEach { (key, item) -> put(key.toString(), toJson(item)) }
  }
  is List<*> -> JSONArray().apply {
    value.forEach { put(toJson(it)) }
  }
  is String, is Boolean, is Int, is Long, is Double -> value
  else -> value.toString()
}

private fun decodeObject(json: String): JSONObject =
  try {
    JSONObject(json)
  } catch (_: Exception) {
    throw ExpoDataFailure("input", null, "", "invalid Domain data options")
  }

private fun JSONObject.optUInt(key: String): UInt? {
  val value = optWhole(key) ?: return null
  if (value > UInt.MAX_VALUE.toLong()) {
    throw ExpoDataFailure("input", null, "", "invalid Domain data options")
  }
  return value.toUInt()
}

private fun JSONObject.optULong(key: String): ULong? = optWhole(key)?.toULong()

private fun JSONObject.optWhole(key: String): Long? {
  if (!has(key) || isNull(key)) return null
  val number = try {
    getDouble(key)
  } catch (_: Exception) {
    throw ExpoDataFailure("input", null, "", "invalid Domain data options")
  }
  if (!number.isFinite() || number < 0.0 || number > Long.MAX_VALUE.toDouble() || number.toLong().toDouble() != number) {
    throw ExpoDataFailure("input", null, "", "invalid Domain data options")
  }
  return number.toLong()
}

private fun JSONObject.optStringList(key: String): List<String> {
  if (!has(key) || isNull(key)) return emptyList()
  val array = try {
    getJSONArray(key)
  } catch (_: Exception) {
    throw ExpoDataFailure("input", null, "", "invalid Domain data options")
  }
  return List(array.length()) { index ->
    try {
      array.getString(index)
    } catch (_: Exception) {
      throw ExpoDataFailure("input", null, "", "invalid Domain data options")
    }
  }
}
