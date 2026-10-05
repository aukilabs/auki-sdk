package expo.modules.aukisdkexpo

import uniffi.auki_sdk_uniffi.AukiDataMetadata
import uniffi.auki_sdk_uniffi.AukiDiscoveryCandidate
import uniffi.auki_sdk_uniffi.AukiDiscoverySource
import uniffi.auki_sdk_uniffi.AukiDomain
import uniffi.auki_sdk_uniffi.AukiDomainPage
import uniffi.auki_sdk_uniffi.AukiDomainSummary
import uniffi.auki_sdk_uniffi.AukiPortal
import uniffi.auki_sdk_uniffi.AukiPortalDomain
import uniffi.auki_sdk_uniffi.AukiPortalPose
import uniffi.auki_sdk_uniffi.AukiStreamEndReason
import uniffi.auki_sdk_uniffi.AukiStreamNext
import uniffi.auki_sdk_uniffi.AukiUrdfLinkTransform

internal fun mapDomain(domain: AukiDomain): Map<String, Any?> = mapOf(
  "id" to domain.id,
  "name" to domain.name,
  "description" to domain.description,
  "organizationId" to domain.organizationId,
)

internal fun mapDomainPage(page: AukiDomainPage): Map<String, Any?> = mapOf(
  "domains" to page.domains.map(::mapDomainSummary),
  "total" to page.total.toDouble(),
  "limit" to page.limit.toInt(),
  "offset" to page.offset.toInt(),
)

internal fun mapDomainSummary(domain: AukiDomainSummary): Map<String, Any?> = mapOf(
  "id" to domain.id,
  "name" to domain.name,
  "organization_id" to domain.organizationId,
)

internal fun mapPortalDomain(value: AukiPortalDomain): Map<String, Any?> = mapOf(
  "id" to value.id,
  "name" to value.name,
  "organization_id" to value.organizationId,
  "is_default" to value.isDefault,
  "added_to_domain_at" to value.addedToDomainAt,
)

internal fun mapPortal(value: AukiPortal): Map<String, Any?> = mapOf(
  "id" to value.id,
  "short_id" to value.shortId,
  "name" to value.name,
  "size" to value.size,
  "organization_id" to value.organizationId,
  "default_domain_id" to value.defaultDomainId,
  "redirect_url" to value.redirectUrl,
  "created_at" to value.createdAt,
  "updated_at" to value.updatedAt,
)

internal fun mapPortalPose(value: AukiPortalPose): Map<String, Any?> = mapOf(
  "id" to value.id,
  "short_id" to value.shortId,
  "domain_id" to value.domainId,
  "reported_size" to value.reportedSize,
  "px" to value.px,
  "py" to value.py,
  "pz" to value.pz,
  "rx" to value.rx,
  "ry" to value.ry,
  "rz" to value.rz,
  "rw" to value.rw,
  "latitude" to value.latitude,
  "longitude" to value.longitude,
  "altitude" to value.altitude,
  "vertical_accuracy" to value.verticalAccuracy,
  "horizontal_accuracy" to value.horizontalAccuracy,
  "gps_timestamp" to value.gpsTimestamp,
  "scanner_device_id" to value.scannerDeviceId,
  "scanner_device_name" to value.scannerDeviceName,
  "scanner_device_model" to value.scannerDeviceModel,
  "placed_at" to value.placedAt,
)

internal fun mapDataMetadata(value: AukiDataMetadata): Map<String, Any?> = mapOf(
  "id" to value.id,
  "domain_id" to value.domainId,
  "name" to value.name,
  "data_type" to value.dataType,
  "size" to value.size.toDouble(),
  "created_at" to value.createdAt,
  "updated_at" to value.updatedAt,
)

internal fun mapCandidate(candidate: AukiDiscoveryCandidate): Map<String, Any?> {
  val mapped = linkedMapOf<String, Any?>(
    "peerId" to candidate.peerId,
    "routes" to candidate.routes,
    "servedProtocols" to candidate.servedProtocols,
    "expiresAt" to candidate.expiresAt,
    "source" to discoverySource(candidate.source),
  )
  candidate.subjectId?.let { mapped["subjectId"] = it }
  candidate.peerType?.let { mapped["peerType"] = it }
  return mapped
}

internal fun encodeUrdfLinks(links: List<AukiUrdfLinkTransform>): String {
  val mapped = links.map { link ->
    mapOf(
      "linkName" to link.linkName,
      "transform" to link.transform.map { it.toDouble() },
      "meshPath" to link.meshPath,
      "colorRgba" to link.colorRgba?.map { it.toDouble() },
    )
  }
  return jsonString(mapped)
}

internal fun encodeStreamNext(next: AukiStreamNext): String = when (next) {
  is AukiStreamNext.Entry -> jsonString(
    mapOf(
      "kind" to "entry",
      "entry" to mapOf(
        "timestampNs" to next.entry.timestampNs.toString(),
        "sequence" to next.entry.sequence.toString(),
        "payloadBase64" to encodeBase64(next.entry.payload),
      ),
    ),
  )
  is AukiStreamNext.End -> jsonString(
    mapOf(
      "kind" to "end",
      "reason" to encodeEndReason(next.reason),
      "entry" to null,
    ),
  )
}

private fun discoverySource(source: AukiDiscoverySource): String = when (source) {
  AukiDiscoverySource.DDS_TRACKER -> "ddsTracker"
}

private fun encodeEndReason(reason: AukiStreamEndReason): Map<String, Any?> = when (reason) {
  is AukiStreamEndReason.SourceEnded -> mapOf("kind" to "source_ended")
  is AukiStreamEndReason.ProducerShuttingDown -> mapOf("kind" to "producer_shutting_down")
  is AukiStreamEndReason.SessionEnded -> mapOf("kind" to "session_ended")
  is AukiStreamEndReason.ProducerError -> mapOf("kind" to "producer_error", "detail" to reason.detail)
}
